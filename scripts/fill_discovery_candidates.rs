#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition = "2024"
[dependencies]
anyhow = "1"
csv = "1"
ttf-parser = "=0.25.1"
---
//! Reviewed translations for the first automatic-discovery batch (2026-10-24).
//! Usage from repository root: <script> <decoded.csv> <translations.csv>
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
};

// Fixed-size map labels use one character. Instrument prefixes use concise
// Chinese to fit their existing bytecode slots; physical units stay unchanged.
const REVIEWED: &[(&str, &str, &str)] = &[
    ("ED7E8A75FDA186D3EB94D96F3F218F96", "End", "终"),
    ("40CB32FF8A1B87F71B653465BEA8C789", "Start", "起"),
    (
        "6EA2DF7413FE7DB37F671AADFC11A046",
        "Removed Type At: ",
        "移除类型索引：",
    ),
    ("787AA81FB0FE24A2E03059C47422FD3B", "AGL: ", "离地"),
    ("09785BB770418707403665E5B5F742A1", "Speed: ", "速度："),
    ("B287673629CE0633A3EC1E9518CF83CA", "mph", "mph"),
    ("948039FBBE1F16E557D366B85C6B468C", "Fuel: ", "燃油"),
    ("5508111BB1448249AFC1BBF19931DFC9", "Throttle: ", "油门："),
    ("3D58BB165025D2B5C2973143FBD27646", "Velocity: ", "速度："),
    ("7F68293DF5484E37E77B8B9E882BF27B", "ft/s", "ft/s"),
    ("9D032756AF87349D84D3A3E809FE46D5", "Speed: ", "速度："),
    ("C64F60D521FB66BE6A6C39F7F7A46B87", "mph", "mph"),
    ("555381FEF23AB99C50D8CE0493D68CE0", "Longitude: ", "经度："),
    (
        "89B1C41318F5B9F2254C33E22DD60C49",
        "  Latitude: ",
        "  纬度：",
    ),
    ("1B938A289345958C3F6F139ED581B3BF", "DAY ", "日"),
];

fn load(path: &std::path::Path) -> Result<Vec<csv::StringRecord>> {
    let mut reader = csv::Reader::from_path(path)?;
    ensure!(
        reader.headers()?.iter().eq([
            "namespace",
            "key",
            "hash",
            "text",
            "trans_text",
            "context_asset"
        ]),
        "Unexpected CSV columns"
    );
    let rows = reader.records().collect::<Result<Vec<_>, _>>()?;
    let mut keys = BTreeSet::new();
    for row in &rows {
        ensure!(
            keys.insert((row[0].to_owned(), row[1].to_owned())),
            "Duplicate CSV key"
        );
    }
    Ok(rows)
}

fn main() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    ensure!(
        args.len() == 2,
        "Usage: fill_discovery_candidates.rs <decoded.csv> <translations.csv>"
    );
    let decoded = load(std::path::Path::new(&args[0]))?;
    let output = std::path::Path::new(&args[1]);
    let mut current = load(output)?;
    let baseline: BTreeMap<_, _> = current
        .iter()
        .map(|r| ((r[0].to_owned(), r[1].to_owned()), r))
        .collect();
    for ((ns, key), row) in &baseline {
        ensure!(
            decoded
                .iter()
                .any(|r| &r[0] == ns && &r[1] == key && r == *row),
            "Existing row changed or missing: {key}"
        );
    }
    let mut additions = Vec::new();
    for &(key, source, translation) in REVIEWED {
        let row = decoded
            .iter()
            .find(|r| &r[0] == "__ASSET_TEXT__" && &r[1] == key)
            .with_context(|| format!("Missing reviewed candidate {key}"))?;
        ensure!(&row[3] == source, "Candidate source changed: {key}");
        if let Some(existing) = baseline.get(&(row[0].to_owned(), key.to_owned())) {
            ensure!(
                &existing[4] == translation,
                "Existing translation differs: {key}"
            );
            continue;
        }
        ensure!(
            row[4].is_empty(),
            "Candidate already has a translation: {key}"
        );
        let filled: csv::StringRecord = row
            .iter()
            .enumerate()
            .map(|(i, field)| if i == 4 { translation } else { field })
            .collect();
        additions.push(filled);
    }
    let added = additions.len();
    current.extend(additions);
    ensure!(
        current.len() == decoded.len(),
        "Unreviewed additional candidates remain"
    );
    ensure!(
        current.iter().all(|r| !r[4].trim().is_empty()),
        "Blank translations remain"
    );
    let font = fs::read("fonts/NNChinese-GB2312-Regular.ttf")?;
    let face = ttf_parser::Face::parse(&font, 0)?;
    let chars: BTreeSet<_> = current
        .iter()
        .flat_map(|r| r[3].chars().chain(r[4].chars()))
        .filter(|c| !c.is_control())
        .collect();
    let missing: Vec<_> = chars
        .iter()
        .filter(|&&c| face.glyph_index(c).is_none())
        .collect();
    ensure!(missing.is_empty(), "Missing glyphs: {missing:?}");
    let mut writer = csv::WriterBuilder::new()
        .quote_style(csv::QuoteStyle::Always)
        .terminator(csv::Terminator::CRLF)
        .from_writer(Vec::new());
    writer.write_record([
        "namespace",
        "key",
        "hash",
        "text",
        "trans_text",
        "context_asset",
    ])?;
    for row in &current {
        writer.write_record(row)?;
    }
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend(writer.into_inner()?);
    fs::write(output, bytes)?;
    println!(
        "Added {added} reviewed rows; {} total; no blank translations; {} characters covered by the font.",
        current.len(),
        chars.len()
    );
    Ok(())
}
