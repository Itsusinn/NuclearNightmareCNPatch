#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition = "2024"
[dependencies]
anyhow = "1"
repak = { git = "https://github.com/trumank/repak", rev = "355b5f62f51959c7cc6dd5a51708646ef483065d", features = ["oodle"] }
---
//! List and extract perk-related assets for localization diagnosis.
use anyhow::Result;
use std::{env, fs, io::BufReader, path::PathBuf};
fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let pak_path = PathBuf::from(args.next().expect("PAK path"));
    let output = PathBuf::from(args.next().expect("output directory"));
    let dll = env::current_dir()?.join("oo2core_9_win64.dll");
    let runtime = env::current_exe()?.with_file_name("oo2core_9_win64.dll");
    if !runtime.is_file() && dll.is_file() { fs::copy(dll, runtime)?; }
    fs::create_dir_all(&output)?;
    let mut reader = BufReader::new(fs::File::open(pak_path)?);
    let pak = repak::PakBuilder::new().reader(&mut reader)?;
    for path in pak.files() {
        let lower = path.to_ascii_lowercase();
        if (lower.contains("/hextree/") || lower.contains("perk")) && lower.ends_with(".uexp") {
            println!("{path}");
            let target = output.join(&path);
            fs::create_dir_all(target.parent().unwrap())?;
            pak.read_file(&path, &mut reader, &mut fs::File::create(target)?)?;
        }
    }
    Ok(())
}
