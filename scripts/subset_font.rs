#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition = "2024"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
csv = "1"
fontdue = "=0.9.5"
png = "=0.17.16"
read-fonts = "=0.46.0"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
skera = "=0.9.0"
textcode = "=0.5.0"
ttf-parser = "=0.25.1"
unicode-general-category = "=1.1.0"
write-fonts = { version = "=0.55.0", default-features = false, features = ["read"] }
---

use anyhow::{Context, Result, ensure};
use clap::Parser;
use fontdue::{Font, FontSettings};
use read_fonts::{
    FontRef, TableProvider,
    collections::IntSet,
    tables::glyf::Glyph,
    types::{GlyphId, Tag},
};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use skera::{Plan, SubsetFlags};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use ttf_parser::Face;
use unicode_general_category::{GeneralCategory, get_general_category};
use write_fonts::FontBuilder;

#[derive(Parser)]
#[command(about = "Build and verify Sarasa Chinese font subsets (pure Rust)")]
struct Args {
    #[arg(long)]
    source: Option<PathBuf>,
    #[arg(long)]
    translations: Option<PathBuf>,
    #[arg(long)]
    output_dir: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Row {
    text: String,
    trans_text: String,
}

fn corpus(reader: impl Read) -> Result<(usize, String, BTreeSet<u32>)> {
    let mut csv = csv::Reader::from_reader(reader);
    let headers = csv.headers()?;
    for field in ["text", "trans_text"] {
        ensure!(headers.iter().any(|h| h == field), "CSV 缺少 {field} 列");
    }
    let mut count = 0;
    let mut text = String::new();
    for row in csv.deserialize::<Row>() {
        let row = row.context("无法读取翻译 CSV")?;
        text.push_str(&row.text);
        text.push_str(&row.trans_text);
        count += 1;
    }
    ensure!(count > 0, "翻译 CSV 没有记录");
    let required = text
        .chars()
        .filter(|&c| printable(c))
        .map(u32::from)
        .collect();
    Ok((count, text, required))
}

fn printable(c: char) -> bool {
    !matches!(
        get_general_category(c),
        GeneralCategory::Control
            | GeneralCategory::Format
            | GeneralCategory::PrivateUse
            | GeneralCategory::Surrogate
            | GeneralCategory::Unassigned
    )
}

fn gb2312() -> BTreeSet<u32> {
    let mut result = BTreeSet::new();
    // Use GB2312, not the larger GBK mapping (which assigns reserved cells).
    for lead in 0xA1..=0xF7 {
        for trail in 0xA1..=0xFE {
            let decoded = textcode::decode::<textcode::Gb2312>([lead, trail]);
            result.extend(
                decoded
                    .chars()
                    .filter(|&c| c != '\0' && c != '\u{FFFD}')
                    .map(u32::from),
            );
        }
    }
    result
}

fn selections(required: &BTreeSet<u32>) -> (BTreeSet<u32>, BTreeSet<u32>) {
    let mut ui = required.clone();
    ui.extend(0x20..0x100);
    let mut common = ui.clone();
    for (start, end) in [
        (0x100, 0x530),
        (0x2000, 0x2070),
        (0x2190, 0x2300),
        (0x25A0, 0x2600),
        (0x3000, 0x3040),
        (0xFF00, 0xFFF0),
    ] {
        common.extend(start..end);
    }
    common.extend(gb2312());
    (ui, common)
}

fn cmap(face: &Face<'_>) -> BTreeSet<u32> {
    let mut result = BTreeSet::new();
    if let Some(cmap) = face.tables().cmap {
        for table in cmap.subtables.into_iter().filter(|t| t.is_unicode()) {
            table.codepoints(|cp| {
                if char::from_u32(cp)
                    .and_then(|c| face.glyph_index(c))
                    .is_some()
                {
                    result.insert(cp);
                }
            });
        }
    }
    result
}

fn check_coverage(available: &BTreeSet<u32>, required: &BTreeSet<u32>) -> Result<()> {
    let missing: Vec<_> = required
        .difference(available)
        .map(|cp| format!("U+{cp:04X}"))
        .collect();
    ensure!(missing.is_empty(), "字体缺少字符：{}", missing.join(", "));
    Ok(())
}

fn hash(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

// Rebuild only the name table, preserving the source's copyright/license and
// other metadata byte-for-byte. All replaced names identify this as a subset.
fn renamed_font(data: &[u8], key: &str, family: &str) -> Result<Vec<u8>> {
    let face = Face::parse(data, 0)?;
    let table = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"name"))
        .context("缺少 name 表")?;
    ensure!(
        table.starts_with(&[0, 0]),
        "不支持带语言标签的 name format 1"
    );
    let version = face
        .names()
        .into_iter()
        .find(|n| n.name_id == 5)
        .and_then(|n| n.to_string())
        .unwrap_or_else(|| "Subset".into());
    let mut records: Vec<((u16, u16, u16, u16), Vec<u8>)> = face
        .names()
        .into_iter()
        .filter(|n| ![1, 2, 3, 4, 6, 13, 14, 16, 17, 18, 21, 22, 25].contains(&n.name_id))
        .map(|n| {
            (
                (
                    n.platform_id as u16,
                    n.encoding_id,
                    n.language_id,
                    n.name_id,
                ),
                n.name.to_vec(),
            )
        })
        .collect();
    for (id, value) in [
        (1, family.to_string()),
        (2, "Regular".into()),
        (3, format!("{family}:{version}:Regular")),
        (4, format!("{family} Regular")),
        (6, format!("NNChinese-{key}-Regular")),
        (16, family.into()),
        (17, "Regular".into()),
        (
            13,
            "Licensed under the SIL Open Font License, Version 1.1. See OFL.txt.".into(),
        ),
        (
            14,
            "https://openfontlicense.org/open-font-license-official-text/".into(),
        ),
    ] {
        records.push((
            (3, 1, 0x409, id),
            value.encode_utf16().flat_map(u16::to_be_bytes).collect(),
        ));
        // New names are ASCII, also valid in Macintosh Roman.
        ensure!(value.is_ascii(), "新字体名称必须为 ASCII");
        records.push(((1, 0, 0, id), value.into_bytes()));
    }
    records.sort_by_key(|r| r.0);
    let mut names = Vec::new();
    let mut strings = Vec::new();
    for value in [
        0,
        u16::try_from(records.len())?,
        u16::try_from(6 + records.len() * 12)?,
    ] {
        names.extend(value.to_be_bytes());
    }
    for ((platform, encoding, language, id), bytes) in records {
        for value in [
            platform,
            encoding,
            language,
            id,
            u16::try_from(bytes.len())?,
            u16::try_from(strings.len())?,
        ] {
            names.extend(value.to_be_bytes());
        }
        strings.extend(bytes);
    }
    names.extend(strings);
    let mut builder = FontBuilder::new();
    builder
        .add_raw(Tag::new(b"name"), names)
        .copy_missing_tables(FontRef::new(data)?);
    Ok(builder.build())
}

fn make_subset(source: &[u8], desired: &BTreeSet<u32>, key: &str, family: &str) -> Result<Vec<u8>> {
    let font = FontRef::new(source)?;
    let unicodes: IntSet<u32> = desired.iter().copied().collect();
    // Retain .notdef/.null/nonmarkingreturn and close composite/GSUB dependencies.
    let gids: IntSet<GlyphId> = (0..3).map(GlyphId::new).collect();
    let plan = Plan::new(
        &gids,
        &unicodes,
        &font,
        SubsetFlags::SUBSET_FLAGS_NOTDEF_OUTLINE
            | SubsetFlags::SUBSET_FLAGS_NAME_LEGACY
            | SubsetFlags::SUBSET_FLAGS_GLYPH_NAMES
            | SubsetFlags::SUBSET_FLAGS_NO_BIDI_CLOSURE,
        &skera::DEFAULT_DROP_TABLES.iter().copied().collect(),
        &IntSet::all(),
        &IntSet::all(),
        &IntSet::all(),
        &IntSet::all(),
    );
    let bytes = skera::subset_font(&font, &plan)?;
    verify_glyphs(source, &bytes, &plan)?;
    renamed_font(&bytes, key, family)
}

#[derive(Default, PartialEq)]
struct Outline(Vec<Vec<f32>>);

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push(vec![0.0, x, y]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push(vec![1.0, x, y]);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.push(vec![2.0, x1, y1, x, y]);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.push(vec![3.0, x1, y1, x2, y2, x, y]);
    }
    fn close(&mut self) {
        self.0.push(vec![4.0]);
    }
}

fn verify_glyphs(source: &[u8], bytes: &[u8], plan: &Plan) -> Result<()> {
    let original = Face::parse(source, 0)?;
    let subset = Face::parse(bytes, 0)?;
    let source = FontRef::new(source)?;
    let target = FontRef::new(bytes)?;
    let (old_loca, old_glyf) = (source.loca(None)?, source.glyf()?);
    let (new_loca, new_glyf) = (target.loca(None)?, target.glyf()?);
    let instructions = |glyph| match glyph {
        None => &[][..],
        Some(Glyph::Simple(g)) => g.instructions(),
        Some(Glyph::Composite(g)) => g.instructions().unwrap_or_default(),
    };
    for (old, new) in plan.old_to_new_glyph_mapping() {
        let old_instructions = instructions(
            old_loca
                .get(old, &old_glyf)
                .context("原字形损坏")?
                .into_glyph(),
        );
        let new_instructions = instructions(
            new_loca
                .get(new, &new_glyf)
                .context("子集字形损坏")?
                .into_glyph(),
        );
        ensure!(
            old_instructions == new_instructions,
            "字形 {old:?} 的 hinting 指令发生变化"
        );
        let mut a = Outline::default();
        let mut b = Outline::default();
        let old_bounds =
            original.outline_glyph(ttf_parser::GlyphId(old.to_u32().try_into()?), &mut a);
        let new_bounds =
            subset.outline_glyph(ttf_parser::GlyphId(new.to_u32().try_into()?), &mut b);
        ensure!(
            old_bounds == new_bounds && a == b,
            "字形 {old:?} 的轮廓发生变化"
        );
    }
    Ok(())
}

fn verify(source: &[u8], bytes: &[u8], desired: &BTreeSet<u32>, family: &str) -> Result<()> {
    let original = Face::parse(source, 0)?;
    let face = Face::parse(bytes, 0)?;
    check_coverage(&cmap(&face), desired)?;
    ensure!(
        face.units_per_em() == original.units_per_em(),
        "unitsPerEm 发生变化"
    );
    for &cp in desired {
        let c = char::from_u32(cp).context("无效码点")?;
        let old = original.glyph_index(c).context("原字体缺字")?;
        let new = face.glyph_index(c).context("子集缺字")?;
        ensure!(
            original.glyph_hor_advance(old) == face.glyph_hor_advance(new)
                && original.glyph_hor_side_bearing(old) == face.glyph_hor_side_bearing(new),
            "U+{cp:04X} 的水平字宽或边距发生变化"
        );
    }
    for tag in [b"fpgm", b"prep", b"cvt ", b"gasp"] {
        let tag = ttf_parser::Tag::from_bytes(tag);
        ensure!(
            original.raw_face().table(tag) == face.raw_face().table(tag),
            "hinting 表 {tag:?} 发生变化"
        );
    }
    ensure!(
        face.names()
            .into_iter()
            .any(|n| n.name_id == 1 && n.to_string().as_deref() == Some(family)),
        "字体名称替换失败"
    );
    // Validate sfnt checksum including checkSumAdjustment.
    ensure!(checksum(bytes) == 0xB1B0_AFBA, "字体总校验和错误");
    Ok(())
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

fn raster_font(bytes: &[u8]) -> Result<Font> {
    Font::from_bytes(bytes, FontSettings::default()).map_err(|e| anyhow::anyhow!(e))
}

fn compare_raster(original: &Font, subset: &Font, sample: &[String]) -> Result<()> {
    for c in sample
        .iter()
        .flat_map(|s| s.chars())
        .collect::<BTreeSet<_>>()
    {
        for size in [16.0, 24.0, 40.0] {
            let (a, pixels_a) = original.rasterize(c, size);
            let (b, pixels_b) = subset.rasterize(c, size);
            ensure!(
                a == b && pixels_a == pixels_b,
                "字符 {c:?} 在 {size}px 下栅格化结果不同"
            );
        }
    }
    Ok(())
}

fn draw(pixels: &mut [u8], font: &Font, text: &str, size: f32, mut x: i32, baseline: i32) {
    for c in text.chars() {
        let (m, bitmap) = font.rasterize(c, size);
        for row in 0..m.height {
            for col in 0..m.width {
                let px = x + m.xmin + col as i32;
                let py = baseline - m.ymin - m.height as i32 + row as i32;
                if (0..1400).contains(&px) && (0..630).contains(&py) {
                    let i = (py as usize * 1400 + px as usize) * 3;
                    let alpha = bitmap[row * m.width + col] as u16;
                    for channel in &mut pixels[i..i + 3] {
                        *channel = (*channel as u16 * (255 - alpha) / 255) as u8;
                    }
                }
            }
        }
        x += m.advance_width.round() as i32;
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("无法定位项目")?;
    let source = args
        .source
        .unwrap_or_else(|| project.join("fonts/SarasaGothicSC-Regular.ttf"));
    let source = source
        .canonicalize()
        .with_context(|| format!("无法读取原字体 {}", source.display()))?;
    let translations = args
        .translations
        .unwrap_or_else(|| project.join("translations.csv"));
    let translations = translations
        .canonicalize()
        .with_context(|| format!("无法读取翻译库 {}", translations.display()))?;
    let output = args
        .output_dir
        .unwrap_or_else(|| project.join("target/font-subsets"));
    fs::create_dir_all(&output)?;
    let output = output.canonicalize()?;
    for name in [
        "NNChinese-UIText-Regular.ttf",
        "NNChinese-GB2312-Regular.ttf",
        "comparison.png",
        "report.json",
    ] {
        let path = output.join(name);
        let resolved = path.canonicalize().unwrap_or(path);
        ensure!(
            resolved != source && resolved != translations,
            "输出路径不能覆盖输入文件"
        );
    }
    let source_bytes = fs::read(&source)?;
    let face =
        Face::parse(&source_bytes, 0).context("无法读取原字体；请确认已执行 git lfs pull")?;
    ensure!(
        face.raw_face()
            .table(ttf_parser::Tag::from_bytes(b"glyf"))
            .is_some()
            && !face.is_variable(),
        "此脚本仅支持静态 TrueType 字体"
    );
    let available = cmap(&face);
    let csv_bytes = fs::read(&translations)?;
    let (rows, text, required) = corpus(csv_bytes.as_slice())?;
    check_coverage(&available, &required)?;
    let (ui, common) = selections(&required);
    let mut seen = BTreeSet::new();
    let sample: Vec<char> = text
        .chars()
        .filter(|c| ('\u{4E00}'..='\u{9FFF}').contains(c) && seen.insert(*c))
        .take(48)
        .collect();
    let split = sample.len().min(24);
    let sample: Vec<String> = vec![
        sample[..split].iter().collect(),
        sample[split..].iter().collect(),
        "ABC xyz 0123456789 + - / % {M} {State}".into(),
    ];
    let original_raster = raster_font(&source_bytes)?;
    let mut pixels = vec![247u8; 1400 * 630 * 3];
    draw(
        &mut pixels,
        &original_raster,
        "Original Sarasa Gothic SC",
        18.0,
        20,
        30,
    );
    for (line, value) in sample.iter().enumerate() {
        draw(
            &mut pixels,
            &original_raster,
            value,
            40.0,
            20,
            85 + line as i32 * 48,
        );
    }
    let mut variants = Vec::new();
    let mut files = Vec::new();
    for (index, (key, selected, family)) in [
        ("UIText", ui, "NN Chinese UI Subset"),
        ("GB2312", common, "NN Chinese Common Subset"),
    ]
    .into_iter()
    .enumerate()
    {
        let desired = selected.intersection(&available).copied().collect();
        let bytes = make_subset(&source_bytes, &desired, key, family)?;
        verify(&source_bytes, &bytes, &desired, family)?;
        let raster = raster_font(&bytes)?;
        compare_raster(&original_raster, &raster, &sample)?;
        let name = format!("NNChinese-{key}-Regular.ttf");
        let top = (index as i32 + 1) * 210;
        draw(&mut pixels, &original_raster, &name, 18.0, 20, top + 30);
        for (line, value) in sample.iter().enumerate() {
            draw(
                &mut pixels,
                &raster,
                value,
                40.0,
                20,
                top + 85 + line as i32 * 48,
            );
        }
        let subset = Face::parse(&bytes, 0)?;
        variants.push(json!({"file": name, "bytes": bytes.len(), "glyphs": subset.number_of_glyphs(),
            "unicode": cmap(&subset).len(), "reduction_percent": (10000.0 * (1.0 - bytes.len() as f64 / source_bytes.len() as f64)).round() / 100.0,
            "missing_corpus_characters": [], "sha256": hash(&bytes)}));
        files.push((name, bytes));
    }
    let report = json!({"source": source, "source_bytes": source_bytes.len(), "source_sha256": hash(&source_bytes),
        "translations_sha256": hash(&csv_bytes), "original_glyphs": face.number_of_glyphs(), "original_unicode": available.len(),
        "corpus_rows": rows, "corpus_characters": required.len(), "variants": variants, "subsetter": "skera 0.9.0",
        "sample_raster_matches_original": true, "rasterizer": "fontdue 0.9.5 (unhinted), 16/24/40 px",
        "horizontal_metrics_match_original": true, "global_hinting_tables_match_original": true,
        "all_retained_outlines_and_hint_instructions_match_original": true});
    let report = serde_json::to_string_pretty(&report)?;
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, 1400, 630);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&pixels)?;
    }
    ensure!(fs::read(&source)? == source_bytes, "运行期间原字体发生变化");
    // Publish only after both variants have passed validation.
    for (name, bytes) in files {
        fs::write(output.join(name), bytes)?;
    }
    fs::write(output.join("comparison.png"), png)?;
    fs::write(output.join("report.json"), &report)?;
    println!("{report}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gb2312_excludes_reserved_cells_and_gbk_extensions() {
        let chars = gb2312();
        assert_eq!(chars.len(), 7445);
        for c in ['中', '文', '・', '―'] {
            assert!(chars.contains(&(c as u32)), "{c}");
        }
        assert!(!chars.contains(&0xFFFD));
        assert!(!chars.contains(&('丂' as u32)));
    }

    #[test]
    fn corpus_handles_bom_multiline_and_control_characters() -> Result<()> {
        let input = "\u{FEFF}text,trans_text\r\n\"Walk\nWASD\",\"行走\u{200B}\u{E000}\"\r\n";
        let (rows, _, chars) = corpus(input.as_bytes())?;
        assert_eq!(rows, 1);
        assert!(chars.contains(&('走' as u32)));
        for c in ['\n', '\u{200B}', '\u{E000}'] {
            assert!(!chars.contains(&(c as u32)));
        }
        assert!(corpus("text,other\nA,B\n".as_bytes()).is_err());
        assert!(corpus("text,trans_text\n".as_bytes()).is_err());
        assert!(check_coverage(&BTreeSet::new(), &chars).is_err());
        Ok(())
    }
}
