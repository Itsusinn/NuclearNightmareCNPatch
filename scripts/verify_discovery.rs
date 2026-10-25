#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition = "2024"
[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
csv = "1"
---
//! Read-only regression comparison; run from the repository root or use absolute paths.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(
    about = "Compare patch build contents and preserve existing translation rows after discovery"
)]
struct Args {
    #[arg(long)]
    before_build: PathBuf,
    #[arg(long)]
    after_build: PathBuf,
    #[arg(long)]
    baseline_csv: PathBuf,
    #[arg(long)]
    decoded_csv: PathBuf,
    /// Optionally require the maintained translation library to equal the baseline.
    #[arg(long)]
    current_csv: Option<PathBuf>,
}

fn files(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("Read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            files(root, &path, out)?;
        } else {
            ensure!(kind.is_file(), "Not a regular file: {}", path.display());
            out.insert(path.strip_prefix(root)?.to_owned(), path);
        }
    }
    Ok(())
}

type Rows = BTreeMap<(String, String), csv::StringRecord>;

fn load(path: &Path) -> Result<(csv::StringRecord, Rows)> {
    let mut reader = csv::Reader::from_path(path)?;
    let header = reader.headers()?.clone();
    let ns = header
        .iter()
        .position(|v| v == "namespace")
        .context("Missing namespace column")?;
    let key = header
        .iter()
        .position(|v| v == "key")
        .context("Missing key column")?;
    for column in ["hash", "text", "trans_text"] {
        ensure!(
            header.iter().any(|v| v == column),
            "Missing {column} column"
        );
    }
    let mut rows = BTreeMap::new();
    for row in reader.records() {
        let row = row?;
        let id = (row[ns].to_owned(), row[key].to_owned());
        ensure!(
            !rows.contains_key(&id),
            "Duplicate key {id:?} in {}",
            path.display()
        );
        rows.insert(id, row);
    }
    ensure!(
        !rows.is_empty(),
        "Empty translation table: {}",
        path.display()
    );
    Ok((header, rows))
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    files(&args.before_build, &args.before_build, &mut before)?;
    files(&args.after_build, &args.after_build, &mut after)?;
    ensure!(!before.is_empty(), "Baseline build is empty");
    ensure!(before.keys().eq(after.keys()), "Build file lists differ");
    for (name, path) in &before {
        ensure!(
            fs::read(path)? == fs::read(&after[name])?,
            "Build content differs: {}",
            name.display()
        );
    }
    println!("All {} generated files are byte-identical.", before.len());

    let (header, baseline) = load(&args.baseline_csv)?;
    if let Some(path) = &args.current_csv {
        let (current_header, current) = load(path)?;
        ensure!(
            header == current_header && baseline == current,
            "Current translations differ from baseline"
        );
    }
    let (decoded_header, decoded) = load(&args.decoded_csv)?;
    ensure!(
        header == decoded_header,
        "Decoded CSV columns differ from baseline"
    );
    for (key, row) in &baseline {
        ensure!(
            decoded.get(key) == Some(row),
            "Lost or changed existing row {key:?}"
        );
    }
    println!(
        "All {} existing records preserved exactly; {} additional candidates discovered.",
        baseline.len(),
        decoded.len() - baseline.len()
    );
    Ok(())
}
