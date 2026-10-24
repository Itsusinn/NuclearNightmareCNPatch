#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition = "2024"
[dependencies]
anyhow = "1"
repak = { git = "https://github.com/trumank/repak", rev = "355b5f62f51959c7cc6dd5a51708646ef483065d", features = ["oodle"] }
---
//! List or extract assets matching case-insensitive path fragments.
use anyhow::{Context, Result};
use std::{env, fs, io::BufReader, path::PathBuf};
fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let pak_path = PathBuf::from(args.next().context("PAK path")?);
    let output = args.next().context("output directory, or - to list only")?;
    let patterns: Vec<_> = args.map(|s| s.to_ascii_lowercase()).collect();
    anyhow::ensure!(!patterns.is_empty(), "provide at least one path fragment");
    let dll = env::current_dir()?.join("oo2core_9_win64.dll");
    let runtime = env::current_exe()?.with_file_name("oo2core_9_win64.dll");
    if !runtime.is_file() && dll.is_file() {
        fs::copy(dll, runtime)?;
    }
    let mut reader = BufReader::new(fs::File::open(pak_path)?);
    let pak = repak::PakBuilder::new().reader(&mut reader)?;
    let mut files = pak.files();
    files.sort();
    for path in files {
        let lower = path.to_ascii_lowercase();
        if patterns.iter().any(|p| lower.contains(p)) {
            println!("{path}");
            if output != "-" {
                anyhow::ensure!(
                    !path.split('/').any(|p| p == "..")
                        && !path.contains(':')
                        && !path.starts_with('/'),
                    "unsafe asset path"
                );
                let target = PathBuf::from(&output).join(&path);
                fs::create_dir_all(target.parent().unwrap())?;
                pak.read_file(&path, &mut reader, &mut fs::File::create(target)?)?;
            }
        }
    }
    Ok(())
}
