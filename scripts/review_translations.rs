#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition = "2024"
[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
csv = "1"
regex = "1"
serde = { version = "1", features = ["derive"] }
tempfile = "3"
ttf-parser = "=0.25.1"
---
//! Generic CSV review workflow. Translation data belongs in CSV, never in this tool.
use anyhow::{Context, Result, ensure};
use clap::{Args, Parser, Subcommand};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const HEADER: [&str; 6] = [
    "namespace",
    "key",
    "hash",
    "text",
    "trans_text",
    "context_asset",
];

#[derive(Parser)]
#[command(about = "Prepare, check and apply translation review CSVs")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate blank review rows for new, changed or untranslated sources.
    Prepare(Inputs),
    /// Validate the complete proposed translation library without writing files.
    Check(Validation),
    /// Validate then atomically write the merged library; review CSV remains intact.
    Apply {
        #[command(flatten)]
        validation: Validation,
        /// Explicit destination; may be the translation library itself.
        #[arg(long)]
        output: PathBuf,
    },
}

#[derive(Args)]
struct Inputs {
    /// Fresh output from nuclear_cn.rs decode, not a candidates report.
    #[arg(long, default_value = "origin.csv")]
    decoded: PathBuf,
    #[arg(long, default_value = "translations.csv")]
    translations: PathBuf,
    #[arg(long, default_value = "review.csv")]
    review: PathBuf,
}

#[derive(Args)]
struct Validation {
    #[command(flatten)]
    inputs: Inputs,
    #[arg(long, default_value = "fonts/NNChinese-GB2312-Regular.ttf")]
    font: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Row {
    namespace: String,
    key: String,
    hash: u32,
    text: String,
    trans_text: String,
    context_asset: String,
}

type Identity = (String, String);
impl Row {
    fn id(&self) -> Identity {
        (self.namespace.clone(), self.key.clone())
    }
    fn same_source(&self, other: &Self) -> bool {
        self.hash == other.hash
            && self.text == other.text
            && (self.namespace != "__ASSET_TEXT__" || self.context_asset == other.context_asset)
    }
}

fn index(rows: &[Row]) -> Result<BTreeMap<Identity, &Row>> {
    let mut result = BTreeMap::new();
    for row in rows {
        ensure!(!row.key.trim().is_empty(), "Empty text key");
        ensure!(
            !row.namespace.chars().any(char::is_control) && !row.key.chars().any(char::is_control),
            "Control character in identity {:?}",
            row.id()
        );
        ensure!(
            !row.text.contains('\0') && !row.trans_text.contains('\0'),
            "NUL in {:?}",
            row.id()
        );
        ensure!(
            row.namespace != "__ASSET_TEXT__" || !row.context_asset.trim().is_empty(),
            "Missing asset context for {:?}",
            row.id()
        );
        ensure!(
            result.insert(row.id(), row).is_none(),
            "Duplicate identity {:?}",
            row.id()
        );
    }
    Ok(result)
}

fn load(path: &Path) -> Result<Vec<Row>> {
    let mut reader =
        csv::Reader::from_path(path).with_context(|| format!("Read {}", path.display()))?;
    ensure!(
        reader.headers()?.iter().eq(HEADER),
        "Unexpected CSV columns in {}",
        path.display()
    );
    let rows = reader
        .deserialize()
        .collect::<std::result::Result<Vec<Row>, _>>()?;
    index(&rows)?;
    Ok(rows)
}

fn load_sources(args: &Inputs) -> Result<(Vec<Row>, Vec<Row>)> {
    let decoded = load(&args.decoded)?;
    let current = load(&args.translations)?;
    ensure!(!decoded.is_empty(), "Decoded source table is empty");
    let live = index(&decoded)?;
    for row in &current {
        ensure!(
            live.contains_key(&row.id()),
            "Existing key missing from decode: {:?}; review removal separately",
            row.id()
        );
    }
    Ok((decoded, current))
}

fn pending(decoded: &[Row], current: &[Row]) -> Result<Vec<Row>> {
    let existing = index(current)?;
    index(decoded)?;
    Ok(decoded
        .iter()
        .filter(|row| {
            existing
                .get(&row.id())
                .is_none_or(|old| !old.same_source(row) || old.trans_text.trim().is_empty())
        })
        .map(|row| {
            let mut row = row.clone();
            row.trans_text.clear();
            row
        })
        .collect())
}

fn merge(decoded: &[Row], current: &[Row], review: &[Row]) -> Result<Vec<Row>> {
    ensure!(!decoded.is_empty(), "Decoded source table is empty");
    let live = index(decoded)?;
    let existing = index(current)?;
    let reviewed = index(review)?;
    for row in current {
        ensure!(
            live.contains_key(&row.id()),
            "Existing key missing from decode: {:?}",
            row.id()
        );
    }
    for row in review {
        let source = live
            .get(&row.id())
            .with_context(|| format!("Unknown review key {:?}", row.id()))?;
        ensure!(
            row.same_source(source),
            "Review source/hash/asset changed: {:?}",
            row.id()
        );
        ensure!(
            !row.trans_text.trim().is_empty(),
            "Blank review translation: {:?}",
            row.id()
        );
    }
    let mut merged = current.to_vec();
    for row in &mut merged {
        if let Some(replacement) = reviewed.get(&row.id()) {
            *row = (*replacement).clone();
        }
    }
    for row in review {
        if !existing.contains_key(&row.id()) {
            merged.push(row.clone());
        }
    }
    let proposed = index(&merged)?;
    for row in decoded {
        let translated = proposed
            .get(&row.id())
            .with_context(|| format!("Unreviewed new source: {:?}", row.id()))?;
        ensure!(
            translated.same_source(row),
            "Unreviewed changed source: {:?}",
            row.id()
        );
        ensure!(
            !translated.trans_text.trim().is_empty(),
            "Blank translation: {:?}",
            row.id()
        );
    }
    Ok(merged)
}

fn validate_text(rows: &[Row], font: &[u8]) -> Result<()> {
    let face = ttf_parser::Face::parse(font, 0)?;
    let placeholders = Regex::new(r"\{[^{}]+\}")?;
    for row in rows {
        ensure!(
            !row.trans_text.trim().is_empty(),
            "Blank translation: {:?}",
            row.id()
        );
        let mut source: Vec<_> = placeholders
            .find_iter(&row.text)
            .map(|m| m.as_str())
            .collect();
        let mut translation: Vec<_> = placeholders
            .find_iter(&row.trans_text)
            .map(|m| m.as_str())
            .collect();
        source.sort_unstable();
        translation.sort_unstable();
        ensure!(
            source == translation,
            "Placeholder names/counts differ: {:?}",
            row.id()
        );
        for c in row.trans_text.chars().filter(|c| !c.is_control()) {
            ensure!(
                face.glyph_index(c).is_some(),
                "Missing font glyph {c:?} in {:?}",
                row.id()
            );
        }
    }
    Ok(())
}

fn write_csv(path: &Path, rows: &[Row]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(b"\xEF\xBB\xBF")?;
    {
        let mut writer = csv::WriterBuilder::new()
            .has_headers(false)
            .quote_style(csv::QuoteStyle::Always)
            .terminator(csv::Terminator::CRLF)
            .from_writer(staged.as_file_mut());
        writer.write_record(HEADER)?;
        for row in rows {
            writer.serialize(row)?;
        }
        writer.flush()?;
    }
    ensure!(load(staged.path())? == rows, "CSV readback mismatch");
    staged.as_file().sync_all()?;
    staged
        .persist(path)
        .with_context(|| format!("Replace {}", path.display()))?;
    Ok(())
}

fn distinct_output(output: &Path, protected: &[&Path]) -> Result<()> {
    if output.exists() {
        let target = output.canonicalize()?;
        for source in protected {
            ensure!(
                target != source.canonicalize()?,
                "Output would overwrite input {}",
                source.display()
            );
        }
    }
    Ok(())
}

fn checked(args: &Validation) -> Result<Vec<Row>> {
    let (decoded, current) = load_sources(&args.inputs)?;
    let review = load(&args.inputs.review)?;
    let merged = merge(&decoded, &current, &review)?;
    validate_text(&merged, &fs::read(&args.font)?)?;
    println!(
        "Validated {} rows ({} review rows); no stale sources, blank translations, placeholder errors or missing glyphs.",
        merged.len(),
        review.len()
    );
    Ok(merged)
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::Prepare(args) => {
            distinct_output(&args.review, &[&args.decoded, &args.translations])?;
            ensure!(
                !args.review.exists() || load(&args.review)?.is_empty(),
                "Review CSV contains rows; retain/archive them before preparing another batch"
            );
            let (decoded, current) = load_sources(&args)?;
            let rows = pending(&decoded, &current)?;
            write_csv(&args.review, &rows)?;
            println!(
                "Prepared {} rows in {}; fill trans_text, then run check/apply.",
                rows.len(),
                args.review.display()
            );
        }
        Command::Check(args) => {
            checked(&args)?;
        }
        Command::Apply { validation, output } => {
            distinct_output(
                &output,
                &[&validation.inputs.decoded, &validation.inputs.review],
            )?;
            let merged = checked(&validation)?;
            write_csv(&output, &merged)?;
            println!(
                "Wrote {}; review CSV retained. Use translations.csv as the authoritative library.",
                output.display()
            );
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    run(Cli::parse().command)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(ns: &str, key: &str, text: &str, translation: &str) -> Row {
        Row {
            namespace: ns.into(),
            key: key.into(),
            hash: 123,
            text: text.into(),
            trans_text: translation.into(),
            context_asset: "Content/UI.uexp".into(),
        }
    }
    #[test]
    fn merge_preserves_other_namespaces_and_accepts_translation_corrections() -> Result<()> {
        let current = vec![
            row("A", "Key", "Old", "旧"),
            row("B", "Key", "Keep", "保留"),
        ];
        let mut decoded = current.clone();
        decoded[0].text = "Changed".into();
        decoded[0].hash = 456;
        decoded.push(row("A", "New", "New", ""));
        let mut review = pending(&decoded, &current)?;
        assert_eq!(review.len(), 2);
        assert!(review.iter().all(|r| r.trans_text.is_empty()));
        review[0].trans_text = "变更".into();
        review[1].trans_text = "新增".into();
        let merged = merge(&decoded, &current, &review)?;
        assert_eq!(merged[1], current[1]);
        assert_eq!(merged[0], review[0]);
        assert_eq!(merge(&decoded, &merged, &review)?, merged);
        let mut correction = merged[1].clone();
        correction.trans_text = "修正译文".into();
        assert_eq!(
            merge(&decoded, &merged, &[correction.clone()])?[1],
            correction
        );
        Ok(())
    }
    #[test]
    fn rejects_unreviewed_changes_stale_reviews_unknown_and_duplicate_keys() {
        let old = row("", "Key", "Old", "旧");
        let mut new = old.clone();
        new.text = "New".into();
        assert!(merge(&[new.clone()], &[old.clone()], &[]).is_err());
        assert!(merge(&[new.clone()], &[old.clone()], &[old.clone()]).is_err());
        new = old.clone();
        new.hash += 1;
        assert!(merge(&[new], &[old.clone()], &[old.clone()]).is_err());
        assert!(merge(&[old.clone()], &[], &[]).is_err());
        assert!(merge(&[old.clone()], &[], &[row("Other", "Key", "Old", "译")]).is_err());
        assert!(index(&[old.clone(), old.clone()]).is_err());
        assert!(merge(&[old.clone()], &[], &[row("", "Key", "Old", " ")]).is_err());
        assert!(merge(&[row("", "Other", "Other", "其他")], &[old.clone()], &[]).is_err());
        assert!(merge(&[], &[old], &[]).is_err());
    }
    #[test]
    fn rejects_relocated_embedded_text_without_review() {
        let old = row("__ASSET_TEXT__", "Key", "Old", "旧");
        let mut new = old.clone();
        new.context_asset = "Content/Other.uexp".into();
        assert!(merge(&[new], &[old.clone()], &[old]).is_err());
    }
    #[test]
    fn validates_repeated_placeholders_and_glyphs() -> Result<()> {
        let font = include_bytes!("../fonts/NNChinese-GB2312-Regular.ttf");
        validate_text(&[row("", "Key", "{x} {x} {y}", "{y} 译 {x} {x}")], font)?;
        assert!(validate_text(&[row("", "Key", "{x} {x}", "{x}")], font).is_err());
        assert!(validate_text(&[row("", "Key", "{x}", "{y}")], font).is_err());
        assert!(validate_text(&[row("", "Key", "Text", "\u{10ffff}")], font).is_err());
        Ok(())
    }
    #[test]
    fn csv_roundtrip_and_failure_leave_existing_output_untouched() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let decoded = dir.path().join("decoded.csv");
        let current = dir.path().join("translations.csv");
        let review = dir.path().join("review.csv");
        let old = row("", "Key", "Quoted, \"text\"\r\nnext", "译文\r\n下一行");
        write_csv(&current, &[old.clone()])?;
        assert_eq!(load(&current)?, vec![old.clone()]);
        write_csv(&review, &[])?;
        assert!(load(&review)?.is_empty());
        let before = fs::read(&current)?;
        let mut changed = old.clone();
        changed.hash += 1;
        write_csv(&decoded, &[changed])?;
        let args = Validation {
            inputs: Inputs {
                decoded,
                translations: current.clone(),
                review,
            },
            font: PathBuf::from("unused.ttf"),
        };
        assert!(
            run(Command::Apply {
                validation: args,
                output: current.clone()
            })
            .is_err()
        );
        assert_eq!(fs::read(current)?, before);
        Ok(())
    }

    #[test]
    fn prepare_protects_pending_review_and_apply_can_replace_library() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let decoded = dir.path().join("decoded.csv");
        let current = dir.path().join("translations.csv");
        let review = dir.path().join("review.csv");
        let font = dir.path().join("font.ttf");
        fs::write(
            &font,
            include_bytes!("../fonts/NNChinese-GB2312-Regular.ttf"),
        )?;
        let old = row("", "Key", "Old", "旧");
        let mut new = row("", "Key", "New", "新");
        new.hash = 456;
        write_csv(&decoded, &[new.clone()])?;
        write_csv(&current, &[old])?;
        let inputs = || Inputs {
            decoded: decoded.clone(),
            translations: current.clone(),
            review: review.clone(),
        };
        run(Command::Prepare(inputs()))?;
        assert!(load(&review)?[0].trans_text.is_empty());
        let before = fs::read(&review)?;
        assert!(run(Command::Prepare(inputs())).is_err());
        assert_eq!(fs::read(&review)?, before);
        write_csv(&review, &[new.clone()])?;
        for _ in 0..2 {
            run(Command::Apply {
                validation: Validation {
                    inputs: inputs(),
                    font: font.clone(),
                },
                output: current.clone(),
            })?;
            assert_eq!(load(&current)?, vec![new.clone()]);
            assert_eq!(load(&review)?, vec![new.clone()]);
        }
        assert!(distinct_output(&review, &[&review]).is_err());
        assert!(distinct_output(&decoded, &[&decoded]).is_err());
        Ok(())
    }

    #[test]
    fn malformed_csv_is_rejected_before_use() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("bad.csv");
        fs::write(
            &path,
            "namespace,key,hash,text,trans_text\n,Key,123,Text,译\n",
        )?;
        assert!(load(&path).is_err());
        fs::write(
            &path,
            "namespace,key,hash,text,trans_text,context_asset\n,Key,bad,Text,译,\n",
        )?;
        assert!(load(&path).is_err());
        let duplicate = "namespace,key,hash,text,trans_text,context_asset\n,Key,123,Text,译,\n,Key,123,Text,译,\n";
        fs::write(&path, duplicate)?;
        assert!(load(&path).is_err());
        Ok(())
    }
}
