#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition = "2024"
[dependencies]
anyhow = "1"
---
//! Read-only inspection of cooked map actor exports and FString offsets.
use anyhow::{Context, Result};
use std::{env, fs, path::PathBuf};
fn u32_at(b: &[u8], p: usize) -> u32 {
    u32::from_le_bytes(b[p..p + 4].try_into().unwrap())
}
fn string(b: &[u8], p: usize) -> Option<(String, usize)> {
    let n = i32::from_le_bytes(b.get(p..p + 4)?.try_into().ok()?);
    if n == 0 || n.unsigned_abs() > 4096 {
        return None;
    }
    let end = p + 4 + n.unsigned_abs() as usize * if n < 0 { 2 } else { 1 };
    let bytes = b.get(p + 4..end)?;
    let s = if n < 0 {
        let units: Vec<_> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if units.last() != Some(&0) {
            return None;
        }
        String::from_utf16(&units[..units.len() - 1]).ok()?
    } else {
        if bytes.last() != Some(&0) {
            return None;
        }
        String::from_utf8(bytes[..bytes.len() - 1].to_vec()).ok()?
    };
    Some((s, end))
}
fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let path = PathBuf::from(args.next().context("map .umap path")?);
    let filter = args.next().unwrap_or_default();
    let h = fs::read(&path)?;
    let d = fs::read(path.with_extension("uexp"))?;
    let summary = string(&h, 52).context("package name")?.1 + 4;
    let mut p = u32_at(&h, summary + 4) as usize;
    let mut names = Vec::new();
    for _ in 0..u32_at(&h, summary) {
        let (s, e) = string(&h, p).context("name")?;
        names.push(s);
        p = e + 4;
    }
    let exports = u32_at(&h, summary + 28) as usize;
    let imports = u32_at(&h, summary + 36) as usize;
    let fname = |p| names[u32_at(&h, p) as usize].as_str();
    for i in 0..u32_at(&h, summary + 24) as usize {
        let at = exports + i * 96;
        let class = u32_at(&h, at) as i32;
        let class_name = if class < 0 {
            fname(imports + (-class as usize - 1) * 32 + 20)
        } else if class > 0 {
            fname(exports + (class as usize - 1) * 96 + 16)
        } else {
            "None"
        };
        if !class_name.contains(&filter) {
            continue;
        }
        let size = i64::from_le_bytes(h[at + 28..at + 36].try_into().unwrap()) as usize;
        let start = i64::from_le_bytes(h[at + 36..at + 44].try_into().unwrap()) as usize - h.len();
        println!(
            "EXPORT {i} {} class={class_name} [{start}..{}]",
            fname(at + 16),
            start + size
        );
        let mut p = start;
        while p + 4 <= start + size {
            if let Some((s, e)) = string(&d[..start + size], p) {
                if s.len() >= 3 && s.chars().all(|c| !c.is_control() || c == '\n' || c == '\r') {
                    println!(
                        "  +{} @{p} {:?} prefix={:02x?}",
                        p - start,
                        s,
                        &d[p.saturating_sub(16).max(start)..p]
                    );
                    p = e;
                    continue;
                }
            }
            p += 1;
        }
    }
    Ok(())
}
