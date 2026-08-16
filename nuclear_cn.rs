#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition = "2024"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
crc32fast = "1"
csv = "1"
encoding_rs = "0.8"
locres-rs = "=1.0.1"
md5 = "0.7"
regex = "1"
repak = { git = "https://github.com/trumank/repak", rev = "355b5f62f51959c7cc6dd5a51708646ef483065d", features = ["oodle"] }
serde = { version = "1", features = ["derive"] }
tempfile = "3"
---

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use crc32fast::Hasher;
use encoding_rs::GBK;
use locres_rs::locres::{self, Locres, LocresEntry, LocresNamespace};
use regex::{bytes::Regex as BytesRegex, Regex};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const ORIGINAL_PAK: &str = "NuclearNightmare/Content/Paks/NuclearNightmare-Windows.pak";
const EN_LOCRES: &str = "NuclearNightmare/Content/Localization/Game/en/Game.locres";
const FONT_SOURCE: &str =
    "NuclearNightmare/Content/Hyper/ResourcePack/Fonts/NotoSans/NotoSansSC-Regular.ufont";
const TUTORIAL_MAP: &str = "NuclearNightmare/Content/Maps/TutorialMap.uexp";
const ASSET_TEXT_NAMESPACE: &str = "__ASSET_TEXT__";
const FONT_TARGETS: &[&str] = &[
    "NuclearNightmare/Content/Fonts/ARIAL.ufont",
    "NuclearNightmare/Content/Fonts/ARIALN.ufont",
    "NuclearNightmare/Content/Fonts/PixelsDream-DemiBold.ufont",
    "NuclearNightmare/Content/Fonts/SELINCAH.ufont",
    "NuclearNightmare/Content/Fonts/SELINCAH_0.ufont",
    "NuclearNightmare/Content/Fonts/Remaster/AlumniSans-Bold.ufont",
    "NuclearNightmare/Content/Fonts/Remaster/AlumniSans-ExtraBold.ufont",
    "NuclearNightmare/Content/Fonts/Remaster/AlumniSans-Light.ufont",
    "NuclearNightmare/Content/Fonts/Remaster/AlumniSans-Medium.ufont",
    "NuclearNightmare/Content/Fonts/Remaster/AlumniSans-Regular.ufont",
    "NuclearNightmare/Content/Fonts/Remaster/AlumniSans-SemiBold.ufont",
];

// 只覆盖 English 槽；Russian 和其他语言继续使用游戏原文件。
const CULTURE_SLOTS: &[&str] = &["en"];

#[derive(Parser)]
#[command(
    name = "nuclear-cn",
    version,
    about = "Nuclear Nightmare 纯 Rust 汉化资源提取/打包工具"
)]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// 从游戏 PAK 提取本地化数据并生成待翻译 CSV
    Decode(DecodeArgs),
    /// 把填写完成的 CSV 编译为可安装的汉化 PAK
    Encode(EncodeArgs),
}

#[derive(Args, Clone)]
struct CommonArgs {
    /// 游戏根目录；默认使用工程目录的上一级
    #[arg(long, default_value = "..")]
    game_dir: PathBuf,
}

#[derive(Args)]
struct DecodeArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// 输出 CSV
    #[arg(short, long, default_value = "origin.csv")]
    output: PathBuf,
    /// 保留中间文件到指定目录，便于排错
    #[arg(long)]
    work_dir: Option<PathBuf>,
}

#[derive(Args)]
struct EncodeArgs {
    #[command(flatten)]
    common: CommonArgs,
    /// 翻译人员填写后的 CSV
    #[arg(short, long, default_value = "origin.csv")]
    input: PathBuf,
    /// 输出 PAK
    #[arg(
        short,
        long,
        default_value = "NuclearNightmare/Content/Paks/zz_NuclearNightmare_Chinese_P.pak"
    )]
    output: PathBuf,
    /// 如有空白 trans_text 则报错；默认以原文回退
    #[arg(long)]
    strict: bool,
    /// 保留构建目录到指定位置
    #[arg(long)]
    work_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CsvRow {
    namespace: String,
    key: String,
    hash: u32,
    text: String,
    trans_text: String,
    context_asset: String,
}

enum Workspace {
    Temporary(TempDir),
    Persistent(PathBuf),
}

impl Workspace {
    fn new(requested: &Option<PathBuf>, prefix: &str) -> Result<Self> {
        if let Some(path) = requested {
            fs::create_dir_all(path)
                .with_context(|| format!("无法创建工作目录 {}", path.display()))?;
            Ok(Self::Persistent(path.clone()))
        } else {
            Ok(Self::Temporary(
                tempfile::Builder::new().prefix(prefix).tempdir()?,
            ))
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::Temporary(dir) => dir.path(),
            Self::Persistent(path) => path,
        }
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Action::Decode(args) => decode(args),
        Action::Encode(args) => encode(args),
    }
}

fn decode(args: DecodeArgs) -> Result<()> {
    let game_dir = resolve_game_dir(&args.common.game_dir)?;
    prepare_oodle(&game_dir)?;
    let pak_path = game_dir.join(ORIGINAL_PAK);
    ensure_file(&pak_path, "游戏主 PAK")?;
    let workspace = Workspace::new(&args.work_dir, "nuclear-cn-decode-")?;
    let root = workspace.path();

    let ui_assets = discover_ui_assets(&pak_path)?;
    let mut wanted = Vec::with_capacity(2 + ui_assets.len());
    wanted.push(EN_LOCRES.to_string());
    wanted.extend(ui_assets.iter().cloned());
    wanted.push(TUTORIAL_MAP.to_string());
    let extracted = extract_entries(&pak_path, &wanted, root)?;

    let locres_path = extracted
        .get(EN_LOCRES)
        .context("内部错误：没有提取英文 Game.locres")?;
    let resource = read_locres(locres_path)?;

    let assets: Vec<(String, PathBuf)> = ui_assets
        .iter()
        .map(|entry| {
            Ok((
                entry.clone(),
                extracted
                    .get(entry)
                    .with_context(|| format!("内部错误：没有提取 {entry}"))?
                    .clone(),
            ))
        })
        .collect::<Result<_>>()?;
    let live = scan_live_ftext(&assets)?;

    let mut rows = Vec::new();
    let mut known_keys = HashSet::new();
    let mut updated_hashes = 0usize;
    for namespace in resource.iter() {
        for record in namespace.iter() {
            known_keys.insert(record.key.clone());
            let (hash, text, context_asset) = match live.get(&record.key) {
                Some((live_text, asset)) => {
                    let live_hash = unreal_text_crc(live_text);
                    if live_hash != record.hash_val {
                        updated_hashes += 1;
                    }
                    (live_hash, live_text.clone(), asset.clone())
                }
                None => (record.hash_val, record.translation.clone(), String::new()),
            };
            rows.push(CsvRow {
                namespace: namespace.name.clone(),
                key: record.key.clone(),
                hash,
                text,
                trans_text: String::new(),
                context_asset,
            });
        }
    }

    let mut inserted = 0usize;
    for (key, (text, asset)) in &live {
        if known_keys.insert(key.clone()) {
            rows.push(CsvRow {
                namespace: String::new(),
                key: key.clone(),
                hash: unreal_text_crc(text),
                text: text.clone(),
                trans_text: String::new(),
                context_asset: asset.clone(),
            });
            inserted += 1;
        }
    }

    let tutorial_path = extracted
        .get(TUTORIAL_MAP)
        .context("内部错误：没有提取教程地图字节码")?;
    let tutorial_data = fs::read(tutorial_path)?;
    let tutorial_strings = scan_blueprint_map_strings(&tutorial_data);
    let mut occurrences = BTreeMap::<String, usize>::new();
    for item in &tutorial_strings {
        let ordinal = occurrences.entry(item.text.clone()).or_default();
        rows.push(CsvRow {
            namespace: ASSET_TEXT_NAMESPACE.to_string(),
            key: asset_text_key(TUTORIAL_MAP, &item.text, *ordinal),
            hash: unreal_text_crc(&item.text),
            text: item.text.clone(),
            trans_text: String::new(),
            context_asset: TUTORIAL_MAP.to_string(),
        });
        *ordinal += 1;
    }

    let preserved = merge_existing_translations(&mut rows, &args.output)?;
    validate_rows(&rows, false)?;
    write_csv(&args.output, &rows)?;

    println!("decode 完成：{}", args.output.display());
    println!("记录数：{}", rows.len());
    println!("LocRes 版本：{:?}", resource.version);
    println!("从当前 UI 资源修正的哈希：{updated_hashes}");
    println!("从当前 UI 资源补入的新记录：{inserted}");
    println!("从教程地图提取的内嵌文本：{}", tutorial_strings.len());
    println!("从现有 CSV 保留的译文：{preserved}");
    println!("请填写 trans_text 列，然后运行 encode。");
    Ok(())
}

fn encode(args: EncodeArgs) -> Result<()> {
    let game_dir = resolve_game_dir(&args.common.game_dir)?;
    prepare_oodle(&game_dir)?;
    let pak_path = game_dir.join(ORIGINAL_PAK);
    ensure_file(&pak_path, "游戏主 PAK")?;
    ensure_file(&args.input, "输入 CSV")?;
    let workspace = Workspace::new(&args.work_dir, "nuclear-cn-encode-")?;
    let root = workspace.path();

    let mut rows = read_csv(&args.input)?;
    validate_rows(&rows, args.strict)?;
    let fallback_count = fill_and_validate_translations(&mut rows)?;

    let encode_entries = vec![
        EN_LOCRES.to_string(),
        FONT_SOURCE.to_string(),
        TUTORIAL_MAP.to_string(),
    ];
    let extracted = extract_entries(&pak_path, &encode_entries, root)?;
    let source_locres = extracted
        .get(EN_LOCRES)
        .context("内部错误：没有提取英文 Game.locres")?;
    let original = read_locres(source_locres)?;
    let translated = build_locres(original.version, &rows);
    let locres_row_count = rows
        .iter()
        .filter(|row| row.namespace != ASSET_TEXT_NAMESPACE)
        .count();
    let translated_path = root.join("Game.zh-Hans.locres");
    write_locres(&translated_path, &translated)?;

    // 立即回读，防止生成一个结构损坏但尚未装包的 locres。
    let verified = read_locres(&translated_path)?;
    if verified.entry_count() != locres_row_count {
        bail!(
            "LocRes 回读校验失败：预期 {} 条，实际 {} 条",
            locres_row_count,
            verified.entry_count()
        );
    }

    let build = root.join("build");
    for culture in CULTURE_SLOTS {
        let target = build
            .join("NuclearNightmare/Content/Localization/Game")
            .join(culture)
            .join("Game.locres");
        copy_to(&translated_path, &target)?;
    }

    let noto = extracted
        .get(FONT_SOURCE)
        .context("内部错误：没有提取中文字体")?;
    for target in FONT_TARGETS {
        copy_to(noto, &build.join(target))?;
    }

    let tutorial_source = extracted
        .get(TUTORIAL_MAP)
        .context("内部错误：没有提取教程地图字节码")?;
    let tutorial_target = build.join(TUTORIAL_MAP);
    let tutorial_patch_count = patch_tutorial_subtitles(tutorial_source, &tutorial_target, &rows)?;

    let metadata = read_pak_metadata(&pak_path)?;
    let staged_pak = root.join("Chinese_P.pak");
    let expected_entries = collect_build_entries(&build)?;
    write_pak(&staged_pak, &build, &expected_entries, &metadata)?;
    verify_pak(&staged_pak, &expected_entries, &metadata)?;

    let output = if args.output.is_absolute() {
        args.output
    } else {
        game_dir.join(args.output)
    };
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(&staged_pak, &output)
        .with_context(|| format!("无法写入最终 PAK {}", output.display()))?;
    verify_pak(&output, &expected_entries, &metadata)?;

    println!("encode 完成：{}", output.display());
    println!("输入记录：{}", rows.len());
    println!("空白译文回退原文：{fallback_count}");
    println!(
        "PAK 版本：{}，mount point：{}，path hash seed：{:?}",
        metadata.version, metadata.mount_point, metadata.path_hash_seed
    );
    println!(
        "仅覆盖 English（en）语言槽，并包含 {} 个中文字体覆盖。",
        FONT_TARGETS.len()
    );
    println!("已从 CSV 对教程地图中的 {tutorial_patch_count} 段内嵌文本进行等长汉化。");
    Ok(())
}

#[derive(Debug)]
struct PakMetadata {
    version: repak::Version,
    mount_point: String,
    path_hash_seed: Option<u64>,
}

fn read_pak_metadata(path: &Path) -> Result<PakMetadata> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let pak = repak::PakBuilder::new()
        .reader(&mut reader)
        .with_context(|| format!("无法读取 PAK 元数据：{}", path.display()))?;
    Ok(PakMetadata {
        version: pak.version(),
        mount_point: pak.mount_point().to_string(),
        path_hash_seed: pak.path_hash_seed(),
    })
}

fn discover_ui_assets(pak_path: &Path) -> Result<Vec<String>> {
    let file = File::open(pak_path)?;
    let mut reader = BufReader::new(file);
    let pak = repak::PakBuilder::new()
        .reader(&mut reader)
        .with_context(|| format!("无法打开 PAK：{}", pak_path.display()))?;
    let mut assets: Vec<String> = pak
        .files()
        .into_iter()
        .filter(|entry| is_ui_asset(entry))
        .collect();
    assets.sort();
    if assets.is_empty() {
        bail!("未在游戏 PAK 中发现可扫描的 UI 资源");
    }
    Ok(assets)
}

fn is_ui_asset(entry: &str) -> bool {
    if !entry.starts_with("NuclearNightmare/Content/") || !entry.ends_with(".uexp") {
        return false;
    }
    let lowered = entry.to_ascii_lowercase();
    let ui_area = lowered.contains("/graphics/")
        || lowered.contains("/antizevoipsystem/widgets/")
        || lowered.contains("/snowmobile/widgets/")
        || lowered.contains("/dv_snowmobile/ui/")
        || lowered.contains("/nicecolorwheel/widgets/")
        || lowered.contains("/hextree/widgets/")
        || lowered.contains("/firstperson/input/actions/");
    ui_area && !lowered.contains("/texture") && !lowered.contains("/materials/")
}

fn extract_entries(
    pak_path: &Path,
    entries: &[String],
    output_dir: &Path,
) -> Result<BTreeMap<String, PathBuf>> {
    let file = File::open(pak_path)?;
    let mut reader = BufReader::new(file);
    let pak = repak::PakBuilder::new()
        .reader(&mut reader)
        .with_context(|| format!("无法打开 PAK：{}", pak_path.display()))?;

    let mut outputs = BTreeMap::new();
    for entry in entries {
        let leaf = Path::new(entry).file_name().context("PAK 条目没有文件名")?;
        let unique = format!("{:08x}-{}", unreal_text_crc(entry), leaf.to_string_lossy());
        let output = output_dir.join(unique);
        let mut writer = BufWriter::new(
            File::create(&output)
                .with_context(|| format!("无法创建提取文件 {}", output.display()))?,
        );
        pak.read_file(entry, &mut reader, &mut writer)
            .with_context(|| format!("无法从 PAK 提取 {entry}"))?;
        writer.flush()?;
        outputs.insert(entry.clone(), output);
    }
    Ok(outputs)
}

fn collect_build_entries(build: &Path) -> Result<Vec<String>> {
    let mut entries = Vec::new();
    collect_build_entries_inner(build, build, &mut entries)?;
    entries.sort();
    Ok(entries)
}

fn collect_build_entries_inner(
    root: &Path,
    current: &Path,
    entries: &mut Vec<String>,
) -> Result<()> {
    for item in fs::read_dir(current)? {
        let path = item?.path();
        if path.is_dir() {
            collect_build_entries_inner(root, &path, entries)?;
        } else if path.is_file() {
            entries.push(path_to_pak_entry(path.strip_prefix(root)?)?);
        }
    }
    Ok(())
}

fn path_to_pak_entry(path: &Path) -> Result<String> {
    let components: Vec<String> = path
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    if components.iter().any(|part| part == "..") {
        bail!("非法 PAK 相对路径：{}", path.display());
    }
    Ok(components.join("/"))
}

fn write_pak(
    output: &Path,
    build: &Path,
    entries: &[String],
    metadata: &PakMetadata,
) -> Result<()> {
    if output.is_file() {
        fs::remove_file(output)
            .with_context(|| format!("无法替换旧的暂存 PAK {}", output.display()))?;
    }
    let writer = BufWriter::new(File::create(output)?);
    let mut pak = repak::PakBuilder::new().writer(
        writer,
        metadata.version,
        metadata.mount_point.clone(),
        metadata.path_hash_seed,
    );
    for entry in entries {
        let data = fs::read(build.join(entry.replace('/', "\\")))?;
        pak.write_file(entry, false, data)
            .with_context(|| format!("无法写入 PAK 条目 {entry}"))?;
    }
    let mut writer = pak.write_index().context("无法写入 PAK 索引")?;
    writer.flush()?;
    Ok(())
}

fn verify_pak(output: &Path, expected: &[String], metadata: &PakMetadata) -> Result<()> {
    let file = File::open(output)?;
    let mut reader = BufReader::new(file);
    let pak = repak::PakBuilder::new()
        .reader(&mut reader)
        .with_context(|| format!("无法回读 PAK：{}", output.display()))?;
    let mut actual = pak.files();
    actual.sort();
    if actual != expected {
        bail!(
            "PAK 内容校验失败：预期 {} 个条目，实际 {} 个条目",
            expected.len(),
            actual.len()
        );
    }
    if pak.version() != metadata.version
        || pak.mount_point() != metadata.mount_point
        || pak.path_hash_seed() != metadata.path_hash_seed
    {
        bail!("PAK 元数据校验失败");
    }
    Ok(())
}

fn read_locres(path: &Path) -> Result<Locres> {
    let path_str = path
        .to_str()
        .with_context(|| format!("LocRes 路径不是有效 UTF-8：{}", path.display()))?;
    locres::read(path_str).with_context(|| format!("无法读取 LocRes：{}", path.display()))
}

fn write_locres(path: &Path, resource: &Locres) -> Result<()> {
    let path_str = path
        .to_str()
        .with_context(|| format!("LocRes 路径不是有效 UTF-8：{}", path.display()))?;
    locres::write(path_str, resource)
        .with_context(|| format!("无法写入 LocRes：{}", path.display()))
}

fn build_locres(version: locres::LocresVer, rows: &[CsvRow]) -> Locres {
    let mut grouped: BTreeMap<String, Vec<&CsvRow>> = BTreeMap::new();
    for row in rows
        .iter()
        .filter(|row| row.namespace != ASSET_TEXT_NAMESPACE)
    {
        grouped.entry(row.namespace.clone()).or_default().push(row);
    }
    let mut output = Locres::new(version);
    for (name, records) in grouped {
        let mut namespace = LocresNamespace::new(name);
        for row in records {
            namespace.insert(LocresEntry {
                key: row.key.clone(),
                translation: row.trans_text.clone(),
                hash_val: row.hash,
            });
        }
        output.insert(namespace);
    }
    output
}

fn read_csv(path: &Path) -> Result<Vec<CsvRow>> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(false)
        .from_path(path)
        .with_context(|| format!("无法读取 {}", path.display()))?;
    reader
        .deserialize::<CsvRow>()
        .enumerate()
        .map(|(index, record)| {
            record.with_context(|| format!("CSV 第 {} 条数据格式错误", index + 2))
        })
        .collect()
}

fn merge_existing_translations(rows: &mut [CsvRow], path: &Path) -> Result<usize> {
    if !path.is_file() {
        return Ok(0);
    }
    let existing = read_csv(path)?;
    let translations: BTreeMap<(String, String), String> = existing
        .into_iter()
        .map(|row| ((row.namespace, row.key), row.trans_text))
        .collect();
    let mut preserved = 0usize;
    for row in rows {
        if let Some(translation) = translations.get(&(row.namespace.clone(), row.key.clone())) {
            if !translation.trim().is_empty() {
                row.trans_text = translation.clone();
                preserved += 1;
            }
        }
    }
    Ok(preserved)
}

fn write_csv(path: &Path, rows: &[CsvRow]) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = File::create(path).with_context(|| format!("无法创建 {}", path.display()))?;
    file.write_all(b"\xEF\xBB\xBF")?;
    let mut writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_writer(file);
    for row in rows {
        writer.serialize(row)?;
    }
    writer.flush()?;
    Ok(())
}

fn fill_and_validate_translations(rows: &mut [CsvRow]) -> Result<usize> {
    let placeholder_re = Regex::new(r"\{[^{}]+\}").unwrap();
    let mut fallback_count = 0usize;
    for (index, row) in rows.iter_mut().enumerate() {
        if row.trans_text.trim().is_empty() {
            row.trans_text = row.text.clone();
            fallback_count += 1;
        }
        let source_tokens: BTreeSet<_> = placeholder_re
            .find_iter(&row.text)
            .map(|capture| capture.as_str())
            .collect();
        let translated_tokens: BTreeSet<_> = placeholder_re
            .find_iter(&row.trans_text)
            .map(|capture| capture.as_str())
            .collect();
        if source_tokens != translated_tokens {
            bail!(
                "CSV 第 {} 条占位符不一致：原文 {:?}，译文 {:?}",
                index + 2,
                source_tokens,
                translated_tokens
            );
        }
    }
    Ok(fallback_count)
}

fn scan_live_ftext(assets: &[(String, PathBuf)]) -> Result<BTreeMap<String, (String, String)>> {
    let key_re = BytesRegex::new(r"\x21\x00\x00\x00([0-9A-F]{32})\x00").unwrap();
    let mut live = BTreeMap::new();
    for (asset_name, path) in assets {
        let data = fs::read(path)?;
        for captures in key_re.captures_iter(&data) {
            let whole = captures.get(0).unwrap();
            let key = std::str::from_utf8(captures.get(1).unwrap().as_bytes())?.to_string();
            let Some(text) = read_fstring(&data, whole.end()) else {
                continue;
            };
            if !text.chars().any(char::is_alphabetic) {
                continue;
            }
            if let Some((previous, previous_asset)) = live.get(&key) {
                if previous != &text {
                    bail!("FText key {key} 在 {previous_asset} 与 {asset_name} 中对应不同原文");
                }
            } else {
                live.insert(key, (text, asset_name.clone()));
            }
        }
    }
    Ok(live)
}

fn read_fstring(data: &[u8], offset: usize) -> Option<String> {
    let length_bytes: [u8; 4] = data.get(offset..offset + 4)?.try_into().ok()?;
    let length = i32::from_le_bytes(length_bytes);
    let start = offset + 4;
    if length == 0 || length.unsigned_abs() > 1_000_000 {
        return None;
    }
    if length > 0 {
        let end = start.checked_add(length as usize)?;
        let bytes = data.get(start..end)?;
        if bytes.last().copied()? != 0 {
            return None;
        }
        String::from_utf8(bytes[..bytes.len() - 1].to_vec()).ok()
    } else {
        let units = length.unsigned_abs() as usize;
        let end = start.checked_add(units.checked_mul(2)?)?;
        let bytes = data.get(start..end)?;
        if !bytes.ends_with(&[0, 0]) {
            return None;
        }
        let utf16: Vec<u16> = bytes[..bytes.len() - 2]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        String::from_utf16(&utf16).ok()
    }
}

fn unreal_text_crc(text: &str) -> u32 {
    let mut hasher = Hasher::new();
    for character in text.chars() {
        hasher.update(&(character as u32).to_le_bytes());
    }
    hasher.finalize()
}

fn validate_rows(rows: &[CsvRow], strict: bool) -> Result<()> {
    if rows.is_empty() {
        bail!("CSV/本地化数据为空");
    }
    let key_re = Regex::new(r"^[0-9A-F]{32}$").unwrap();
    let mut seen = HashSet::new();
    let mut blank = 0usize;
    for (index, row) in rows.iter().enumerate() {
        if !key_re.is_match(&row.key) {
            bail!("第 {} 条记录的 key 无效：{}", index + 2, row.key);
        }
        if row.text.is_empty() {
            bail!("第 {} 条记录的 text 为空", index + 2);
        }
        if row.namespace == ASSET_TEXT_NAMESPACE && row.context_asset != TUTORIAL_MAP {
            bail!(
                "第 {} 条资源内嵌文本指向不受支持的资产：{}",
                index + 2,
                row.context_asset
            );
        }
        if !seen.insert((row.namespace.clone(), row.key.clone())) {
            bail!("第 {} 条记录的 namespace/key 重复", index + 2);
        }
        if row.trans_text.trim().is_empty() {
            blank += 1;
        }
    }
    if strict && blank > 0 {
        bail!("strict 模式中 trans_text 不得留空；当前有 {blank} 条空白译文");
    }
    Ok(())
}

fn copy_to(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, target)
        .with_context(|| format!("无法复制 {} -> {}", source.display(), target.display()))?;
    Ok(())
}

#[derive(Clone, Copy)]
enum BlueprintStringEncoding {
    Ansi,
    Unicode,
}

struct BlueprintMapString {
    start: usize,
    text: String,
    encoding: BlueprintStringEncoding,
}

fn is_user_facing_map_text(text: &str) -> bool {
    let character_count = text.chars().count();
    let ascii_letters = text
        .bytes()
        .filter(|byte| byte.is_ascii_alphabetic())
        .count();
    character_count >= 20
        && text.split_whitespace().count() >= 4
        && ascii_letters >= 10
        && ascii_letters * 2 >= character_count
}

fn scan_blueprint_map_strings(data: &[u8]) -> Vec<BlueprintMapString> {
    let mut output = Vec::new();
    let mut opcode = 0usize;
    while opcode + 2 < data.len() {
        match data[opcode] {
            0x1f => {
                let start = opcode + 1;
                let mut end = start;
                while end < data.len() && end - start <= 4096 {
                    let byte = data[end];
                    if byte == 0 {
                        break;
                    }
                    if !matches!(byte, b'\t' | b'\n' | b'\r' | 0x20..=0x7e) {
                        break;
                    }
                    end += 1;
                }
                if end < data.len() && data[end] == 0 {
                    if let Ok(text) = std::str::from_utf8(&data[start..end]) {
                        if is_user_facing_map_text(text) {
                            output.push(BlueprintMapString {
                                start,
                                text: text.to_string(),
                                encoding: BlueprintStringEncoding::Ansi,
                            });
                            opcode = end + 1;
                            continue;
                        }
                    }
                }
            }
            0x34 => {
                let start = opcode + 1;
                let mut end = start;
                while end + 1 < data.len() && end - start <= 8192 {
                    if data[end] == 0 && data[end + 1] == 0 {
                        break;
                    }
                    end += 2;
                }
                if end + 1 < data.len() && data[end] == 0 && data[end + 1] == 0 {
                    let units: Vec<u16> = data[start..end]
                        .chunks_exact(2)
                        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                        .collect();
                    if let Ok(text) = String::from_utf16(&units) {
                        if is_user_facing_map_text(&text) {
                            output.push(BlueprintMapString {
                                start,
                                text,
                                encoding: BlueprintStringEncoding::Unicode,
                            });
                            opcode = end + 2;
                            continue;
                        }
                    }
                }
            }
            _ => {}
        }
        opcode += 1;
    }
    output
}

fn asset_text_key(asset: &str, text: &str, ordinal: usize) -> String {
    format!("{:X}", md5::compute(format!("{asset}\0{text}\0{ordinal}")))
}

fn patch_tutorial_subtitles(source: &Path, target: &Path, rows: &[CsvRow]) -> Result<usize> {
    let mut data = fs::read(source)?;
    let original_len = data.len();
    let strings = scan_blueprint_map_strings(&data);
    let mut occurrences = BTreeMap::<String, usize>::new();
    let map_rows: BTreeMap<&str, &CsvRow> = rows
        .iter()
        .filter(|row| row.namespace == ASSET_TEXT_NAMESPACE && row.context_asset == TUTORIAL_MAP)
        .map(|row| (row.key.as_str(), row))
        .collect();
    if map_rows.len() != strings.len() {
        bail!(
            "教程地图 CSV 记录数不匹配：CSV {} 条，资源 {} 条；请重新运行 decode",
            map_rows.len(),
            strings.len()
        );
    }
    for item in &strings {
        let ordinal = occurrences.entry(item.text.clone()).or_default();
        let key = asset_text_key(TUTORIAL_MAP, &item.text, *ordinal);
        *ordinal += 1;
        let row = map_rows
            .get(key.as_str())
            .with_context(|| format!("教程地图 CSV 缺少文本：{}", item.text))?;
        if row.text != item.text || row.hash != unreal_text_crc(&item.text) {
            bail!(
                "教程地图 CSV 原文或哈希已失效：{}；请重新运行 decode",
                item.text
            );
        }
        replace_blueprint_string(&mut data, item, &row.trans_text)?;
    }
    if data.len() != original_len {
        bail!("教程地图等长替换校验失败");
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(target, data)
        .with_context(|| format!("无法写入教程地图文本补丁 {}", target.display()))?;
    Ok(strings.len())
}

fn replace_blueprint_string(
    data: &mut [u8],
    item: &BlueprintMapString,
    chinese: &str,
) -> Result<()> {
    let start = item.start;
    let english = &item.text;
    if start == 0 {
        bail!("教程地图文本偏移无效：{english}");
    }
    let slot_len = match item.encoding {
        BlueprintStringEncoding::Ansi => {
            if data[start - 1] != 0x1f
                || data.get(start..start + english.len()) != Some(english.as_bytes())
                || data.get(start + english.len()) != Some(&0)
            {
                bail!("教程地图文本不是预期的 Blueprint ANSI 字符串：{english}");
            }
            english.len() + 1
        }
        BlueprintStringEncoding::Unicode => {
            let original: Vec<u8> = english
                .encode_utf16()
                .chain(std::iter::once(0))
                .flat_map(u16::to_le_bytes)
                .collect();
            if data[start - 1] != 0x34
                || data.get(start..start + original.len()) != Some(original.as_slice())
            {
                bail!("教程地图文本不是预期的 Blueprint Unicode 字符串：{english}");
            }
            original.len()
        }
    };
    if matches!(item.encoding, BlueprintStringEncoding::Ansi) && slot_len % 2 != 0 {
        let (encoded, _, had_errors) = GBK.encode(chinese);
        if had_errors || encoded.len() + 1 > slot_len {
            bail!("教程地图译文无法等长编码为 GBK：{chinese}");
        }
        let slot = &mut data[start..start + slot_len];
        slot.fill(b' ');
        slot[..encoded.len()].copy_from_slice(&encoded);
        slot[slot_len - 1] = 0;
        return Ok(());
    }
    let slot_units = slot_len / 2;
    let translated: Vec<u16> = chinese.encode_utf16().collect();
    if translated.len() + 1 > slot_units {
        bail!("教程地图译文过长，无法保持字节码偏移：{chinese}");
    }
    let mut bytes = Vec::with_capacity(slot_len);
    for unit in translated
        .into_iter()
        .chain(std::iter::repeat_n(' ' as u16, slot_units - 1))
        .take(slot_units - 1)
        .chain(std::iter::once(0))
    {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    data[start - 1] = 0x34; // EX_UnicodeStringConst（ANSI 偶数字节槽会转换为 Unicode）
    data[start..start + slot_len].copy_from_slice(&bytes);
    Ok(())
}

fn resolve_game_dir(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).with_context(|| format!("游戏目录不存在：{}", path.display()))
}

/// repak 在进程内动态加载 Oodle。其 loader 默认在 Rust 可执行文件旁查找，
/// 缺失时才联网下载；这里优先复用游戏目录或旧工具目录中的运行库，以支持离线运行。
fn prepare_oodle(game_dir: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        const DLL: &str = "oo2core_9_win64.dll";
        let executable = std::env::current_exe().context("无法确定 Rust CLI 可执行文件位置")?;
        let target = executable.with_file_name(DLL);
        if target.is_file() {
            return Ok(());
        }
        let candidates = [
            std::env::current_dir()
                .context("无法确定当前工作目录")?
                .join(DLL),
            game_dir.join(DLL),
            game_dir.join("Engine/Binaries/Win64").join(DLL),
            game_dir.join("_cn_patch_tools/repak").join(DLL),
        ];
        if let Some(source) = candidates.iter().find(|path| path.is_file()) {
            fs::copy(source, &target).with_context(|| {
                format!(
                    "无法准备 Oodle 运行库 {} -> {}",
                    source.display(),
                    target.display()
                )
            })?;
        }
    }
    Ok(())
}

fn ensure_file(path: &Path, label: &str) -> Result<()> {
    if !path.is_file() {
        bail!("找不到{label}：{}", path.display());
    }
    Ok(())
}
