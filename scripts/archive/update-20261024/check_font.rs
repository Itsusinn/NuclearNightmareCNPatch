#!/usr/bin/env -S cargo +nightly -Zscript
---
[dependencies]
csv = "1"
ttf-parser = "=0.25.1"
---
use std::{collections::BTreeSet, error::Error, fs};
fn main() -> Result<(), Box<dyn Error>> {
    let current = csv::Reader::from_path("target/update-20261024/current.csv")?.records().collect::<Result<Vec<_>, _>>()?;
    let final_rows = csv::Reader::from_path("translations.csv")?.records().collect::<Result<Vec<_>, _>>()?;
    assert_eq!(current, final_rows, "CSV formatting changed record content");
    println!("All {} final CSV records match the strict build input", final_rows.len());
    let data = fs::read("fonts/NNChinese-GB2312-Regular.ttf")?;
    let face = ttf_parser::Face::parse(&data, 0)?;
    let mut chars = BTreeSet::new();
    for row in csv::Reader::from_path("translations.csv")?.records() {
        let row = row?;
        chars.extend(row[3].chars().chain(row[4].chars()).filter(|c| !c.is_control()));
    }
    let missing: Vec<_> = chars.iter().filter(|&&c| face.glyph_index(c).is_none()).collect();
    if !missing.is_empty() { return Err(format!("Missing glyphs: {missing:?}").into()); }
    println!("Font coverage passed: {} distinct source/translation characters", chars.len());
    Ok(())
}
