#!/usr/bin/env -S cargo +nightly -Zscript
---
[package]
edition = "2024"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
crc32fast = "1"
csv = "1"
locres-rs = "=1.0.1"
md5 = "0.7"
regex = "1"
repak = { git = "https://github.com/trumank/repak", rev = "355b5f62f51959c7cc6dd5a51708646ef483065d", features = ["oodle"] }
serde = { version = "1", features = ["derive"] }
tempfile = "3"
---

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use crc32fast::Hasher;
use locres_rs::locres::{self, Locres, LocresEntry, LocresNamespace};
use regex::{Regex, bytes::Regex as BytesRegex};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const ORIGINAL_PAK: &str = "NuclearNightmare/Content/Paks/NuclearNightmare-Windows.pak";
const EN_LOCRES: &str = "NuclearNightmare/Content/Localization/Game/en/Game.locres";
const FONT_TEMPLATE: &str =
    "NuclearNightmare/Content/Hyper/ResourcePack/Fonts/NotoSans/NotoSansSC-Regular.ufont";
const SARASA_FONT: &[u8] = include_bytes!("../../../fonts/NNChinese-GB2312-Regular.ttf");
const SARASA_LICENSE: &[u8] = include_bytes!("../../../fonts/OFL.txt");
const SARASA_NOTICE: &[u8] = include_bytes!("../../../fonts/NOTICE.txt");
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
    /// 禁用 Zlib 压缩，生成无压缩 PAK
    #[arg(long)]
    no_compression: bool,
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

fn original_main() -> Result<()> {
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
    let embedded_paths = discover_embedded_paths(&pak_path)?;
    wanted.push(TUTORIAL_MAP.replace(".uexp", ".umap"));
    for path in &embedded_paths {
        wanted.push(path.clone());
        wanted.push(path.replace(".uexp", ".uasset"));
    }
    wanted.sort();
    wanted.dedup();
    let extracted = extract_entries(&pak_path, &wanted, root)?;
    let (scanned_assets, candidates) = scan_embedded_assets(&extracted, &embedded_paths)?;
    let candidates_path = args.output.with_extension("candidates.csv");
    write_candidates(&candidates_path, &candidates)?;

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
            let identity = (namespace.name.clone(), record.key.clone());
            known_keys.insert(identity.clone());
            let (hash, text, context_asset) = match live.get(&identity) {
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
    for ((namespace, key), (text, asset)) in &live {
        if known_keys.insert((namespace.clone(), key.clone())) {
            rows.push(CsvRow {
                namespace: namespace.clone(),
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
    let tutorial_header = fs::read(
        extracted
            .get(&TUTORIAL_MAP.replace(".uexp", ".umap"))
            .context("缺少教程地图头")?,
    )?;
    let tutorial_strings = scan_tutorial_strings(&tutorial_header, &tutorial_data)?;
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

    rows.extend(embedded_ui_rows(&extracted, &scanned_assets)?);
    let bundled: Vec<CsvRow> =
        csv::Reader::from_reader(include_bytes!("../../../translations.csv").as_slice())
            .deserialize()
            .collect::<std::result::Result<_, _>>()?;
    let seeded = merge_translations(&mut rows, bundled);
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
    println!("从随项目提供的翻译库匹配：{seeded}");
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

    let mut encode_entries = vec![
        EN_LOCRES.to_string(),
        FONT_TEMPLATE.to_string(),
        TUTORIAL_MAP.to_string(),
    ];
    encode_entries.push(TUTORIAL_MAP.replace(".uexp", ".umap"));
    let mut embedded_paths: Vec<String> = rows
        .iter()
        .filter(|row| row.namespace == ASSET_TEXT_NAMESPACE && row.context_asset != TUTORIAL_MAP)
        .map(|row| row.context_asset.clone())
        .collect();
    embedded_paths.sort();
    embedded_paths.dedup();
    for path in &embedded_paths {
        encode_entries.push(path.clone());
        encode_entries.push(path.replace(".uexp", ".uasset"));
    }
    let extracted = extract_entries(&pak_path, &encode_entries, root)?;
    let (scanned_assets, candidates) = scan_embedded_assets(&extracted, &embedded_paths)?;
    write_candidates(&root.join("candidates.csv"), &candidates)?;
    validate_discovered_rows(&rows, &scanned_assets)?;
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

    let font_template = extracted
        .get(FONT_TEMPLATE)
        .context("内部错误：没有提取字体容器模板")?;
    let font_override = build_font_override(&fs::read(font_template)?, SARASA_FONT)?;
    for target in FONT_TARGETS {
        let path = build.join(target);
        fs::create_dir_all(path.parent().context("字体目标路径缺少父目录")?)?;
        fs::write(path, &font_override)?;
    }
    let notices = build.join("NuclearNightmare/Content/Localization/ThirdParty/SarasaGothic");
    fs::create_dir_all(&notices)?;
    fs::write(notices.join("OFL.txt"), SARASA_LICENSE)?;
    fs::write(notices.join("NOTICE.txt"), SARASA_NOTICE)?;

    patch_embedded_ui(&extracted, &build, &rows, &scanned_assets)?;
    let tutorial_source = extracted
        .get(TUTORIAL_MAP)
        .context("内部错误：没有提取教程地图字节码")?;
    let tutorial_target = build.join(TUTORIAL_MAP);
    let tutorial_header = extracted
        .get(&TUTORIAL_MAP.replace(".uexp", ".umap"))
        .context("缺少教程地图头")?;
    let tutorial_patch_count =
        patch_tutorial_subtitles(tutorial_source, tutorial_header, &tutorial_target, &rows)?;

    let metadata = read_pak_metadata(&pak_path)?;
    let staged_pak = root.join("Chinese_P.pak");
    let expected_entries = collect_build_entries(&build)?;
    write_pak(
        &staged_pak,
        &build,
        &expected_entries,
        &metadata,
        !args.no_compression,
    )?;
    verify_pak(&staged_pak, &expected_entries, &metadata, &build)?;

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
    verify_pak(&output, &expected_entries, &metadata, &build)?;

    println!("encode 完成：{}", output.display());
    println!("输入记录：{}", rows.len());
    println!("空白译文回退原文：{fallback_count}");
    println!(
        "PAK 压缩：{}",
        if args.no_compression { "无" } else { "Zlib" }
    );
    println!(
        "PAK 版本：{}，mount point：{}，path hash seed：{:?}",
        metadata.version, metadata.mount_point, metadata.path_hash_seed
    );
    println!(
        "仅覆盖 English（en）语言槽，并包含 {} 个 Sarasa 常用中文子集字体覆盖。",
        FONT_TARGETS.len()
    );
    println!("已从 CSV 对教程地图中的 {tutorial_patch_count} 段内嵌文本进行等长汉化。");
    println!(
        "已汉化物品操作提示和通知：{} 个资源，{} 段内嵌文本。",
        scanned_assets.len(),
        scanned_assets
            .iter()
            .map(|asset| asset.literals.len())
            .sum::<usize>()
    );
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

fn build_font_override(template: &[u8], font: &[u8]) -> Result<Vec<u8>> {
    // This game's cooked .ufont wraps the sfnt payload with a little-endian
    // i32 byte count and a zero i32 trailer. Reject changed formats on updates.
    let length = template.get(..4).context("字体容器模板缺少长度字段")?;
    let length = i32::from_le_bytes(length.try_into()?);
    if length < 4
        || template.len() != length as usize + 8
        || template.get(template.len() - 4..) != Some(&[0, 0, 0, 0][..])
    {
        bail!("原游戏 .ufont 容器格式发生变化，不能安全替换字体");
    }
    let valid_signature =
        |bytes: &[u8]| bytes.starts_with(b"OTTO") || bytes.starts_with(&[0, 1, 0, 0]);
    if !valid_signature(&template[4..]) || !valid_signature(font) {
        bail!("字体数据不是受支持的 TTF/OTF sfnt 格式");
    }
    let font_length = i32::try_from(font.len()).context("字体超过 .ufont 长度上限")?;
    let mut output = Vec::with_capacity(font.len() + 8);
    output.extend_from_slice(&font_length.to_le_bytes());
    output.extend_from_slice(font);
    output.extend_from_slice(&[0; 4]);
    Ok(output)
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
    compress: bool,
) -> Result<()> {
    if output.is_file() {
        fs::remove_file(output)
            .with_context(|| format!("无法替换旧的暂存 PAK {}", output.display()))?;
    }
    let writer = BufWriter::new(File::create(output)?);
    let mut pak = repak::PakBuilder::new()
        .compression([repak::Compression::Zlib])
        .writer(
            writer,
            metadata.version,
            metadata.mount_point.clone(),
            metadata.path_hash_seed,
        );
    for entry in entries {
        let data = fs::read(build.join(entry.replace('/', "\\")))?;
        pak.write_file(entry, compress, data)
            .with_context(|| format!("无法写入 PAK 条目 {entry}"))?;
    }
    let mut writer = pak.write_index().context("无法写入 PAK 索引")?;
    writer.flush()?;
    Ok(())
}

fn verify_pak(
    output: &Path,
    expected: &[String],
    metadata: &PakMetadata,
    build: &Path,
) -> Result<()> {
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
    for entry in expected {
        let unpacked = pak
            .get(entry, &mut reader)
            .with_context(|| format!("无法解压回读 PAK 条目 {entry}"))?;
        let source = fs::read(build.join(entry))?;
        if unpacked != source {
            bail!("PAK 条目内容校验失败：{entry}");
        }
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
    Ok(merge_translations(rows, read_csv(path)?))
}

fn merge_translations(rows: &mut [CsvRow], existing: Vec<CsvRow>) -> usize {
    let translations: BTreeMap<_, _> = existing
        .into_iter()
        .map(|row| ((row.namespace.clone(), row.key.clone()), row))
        .collect();
    let mut preserved = 0usize;
    for row in rows {
        if let Some(previous) = translations.get(&(row.namespace.clone(), row.key.clone())) {
            // 更新了原文的键不能悄悄继承旧译文；允许原 locres 的首尾空白差异。
            if previous.hash == row.hash
                && previous.text.trim() == row.text.trim()
                && !previous.trans_text.trim().is_empty()
            {
                row.trans_text = previous.trans_text.clone();
                preserved += 1;
            }
        }
    }
    preserved
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
        let mut source_tokens: Vec<_> = placeholder_re
            .find_iter(&row.text)
            .map(|capture| capture.as_str())
            .collect();
        let mut translated_tokens: Vec<_> = placeholder_re
            .find_iter(&row.trans_text)
            .map(|capture| capture.as_str())
            .collect();
        source_tokens.sort_unstable();
        translated_tokens.sort_unstable();
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

type LiveTexts = BTreeMap<(String, String), (String, String)>;

fn scan_live_ftext(assets: &[(String, PathBuf)]) -> Result<LiveTexts> {
    // Find identifier FStrings first, then verify the preceding namespace and Base
    // history header. Searching every zero-filled header is prohibitively slow in
    // cooked UI assets that also embed large textures.
    let base_re =
        BytesRegex::new(r"(?-u:[\x00-\xff][\x00-\x03]\x00\x00[A-Za-z0-9_./:-]{1,1023}\x00)")?;
    let script_re = BytesRegex::new(r"(?-u:\x29\x01[\x1f\x34])")?;
    let mut live = BTreeMap::new();
    for (asset_name, path) in assets {
        let data = fs::read(path)?;
        scan_text_data(&data, asset_name, &base_re, &script_re, &mut live)?;
    }
    Ok(live)
}

fn scan_text_data(
    data: &[u8],
    asset: &str,
    base_re: &BytesRegex,
    script_re: &BytesRegex,
    live: &mut LiveTexts,
) -> Result<()> {
    for found in base_re.find_iter(data) {
        if let Some((namespace, key, text)) = read_base_text_at_key(data, found.start()) {
            insert_live_text(live, namespace, key, text, asset)?;
        }
    }
    for found in script_re.find_iter(data) {
        // EX_TextConst, LocalizedText: source, key, namespace (string expressions).
        if let Some((text, end)) = read_script_string(data, found.start() + 2) {
            if let Some((key, end)) = read_script_string(data, end) {
                if let Some((namespace, _)) = read_script_string(data, end) {
                    insert_live_text(live, namespace, key, text, asset)?;
                }
            }
        }
    }
    Ok(())
}

fn valid_text_identity(namespace: &str, key: &str) -> bool {
    namespace.len() <= 1024
        && !namespace.chars().any(char::is_control)
        && !key.is_empty()
        && key.len() <= 1024
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_./:-".contains(&b))
        && (key.bytes().any(|b| b.is_ascii_alphabetic())
            || (key.len() == 32 && key.bytes().all(|b| b.is_ascii_digit())))
}

fn insert_live_text(
    live: &mut LiveTexts,
    namespace: String,
    key: String,
    text: String,
    asset: &str,
) -> Result<()> {
    if !valid_text_identity(&namespace, &key)
        || text.is_empty()
        || text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Ok(());
    }
    let identity = (namespace, key);
    if let Some((previous, previous_asset)) = live.get(&identity) {
        if previous != &text {
            bail!(
                "FText {:?} 在 {previous_asset} 与 {asset} 中对应不同原文",
                identity
            );
        }
    } else {
        live.insert(identity, (text, asset.to_string()));
    }
    Ok(())
}

fn read_base_text_at_key(data: &[u8], offset: usize) -> Option<(String, String, String)> {
    let (key, end) = read_fstring_at(data, offset)?;
    if !valid_text_identity("", &key) {
        return None;
    }
    for span in 4..=offset.min(2054) {
        let start = offset - span;
        let Some(header) = start.checked_sub(5).and_then(|n| data.get(n..start)) else {
            continue;
        };
        if header[0] > 0x1f || header[1..] != [0, 0, 0, 0] {
            continue;
        }
        let len = i32::from_le_bytes(data.get(start..start + 4)?.try_into().ok()?);
        let bytes = (len.unsigned_abs() as usize).checked_mul(if len < 0 { 2 } else { 1 })?;
        if bytes != span - 4 {
            continue;
        }
        let Some((namespace, ns_end)) = read_fstring_at(data, start) else {
            continue;
        };
        if ns_end == offset && valid_text_identity(&namespace, &key) {
            let (text, _) = read_fstring_at(data, end)?;
            return Some((namespace, key, text));
        }
    }
    None
}

fn read_script_string(data: &[u8], offset: usize) -> Option<(String, usize)> {
    let start = offset.checked_add(1)?;
    match data.get(offset)? {
        0x1f => {
            let bytes = data.get(start..)?;
            let len = bytes.iter().take(65536).position(|b| *b == 0)?;
            Some((
                std::str::from_utf8(&bytes[..len]).ok()?.to_string(),
                start + len + 1,
            ))
        }
        0x34 => {
            let mut units = Vec::new();
            for pair in data.get(start..)?.chunks_exact(2).take(65536) {
                let unit = u16::from_le_bytes([pair[0], pair[1]]);
                if unit == 0 {
                    return Some((
                        String::from_utf16(&units).ok()?,
                        start + (units.len() + 1) * 2,
                    ));
                }
                units.push(unit);
            }
            None
        }
        _ => None,
    }
}

fn read_fstring_at(data: &[u8], offset: usize) -> Option<(String, usize)> {
    let length_bytes: [u8; 4] = data.get(offset..offset + 4)?.try_into().ok()?;
    let length = i32::from_le_bytes(length_bytes);
    let start = offset + 4;
    if length == 0 {
        return Some((String::new(), start));
    }
    if length.unsigned_abs() > 1_000_000 {
        return None;
    }
    if length > 0 {
        let end = start.checked_add(length as usize)?;
        let bytes = data.get(start..end)?;
        if bytes.last().copied()? != 0 {
            return None;
        }
        Some((
            String::from_utf8(bytes[..bytes.len() - 1].to_vec()).ok()?,
            end,
        ))
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
        Some((String::from_utf16(&utf16).ok()?, end))
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
    let mut seen = HashSet::new();
    let mut blank = 0usize;
    for (index, row) in rows.iter().enumerate() {
        if !valid_text_identity(&row.namespace, &row.key) {
            bail!("第 {} 条记录的 key 无效：{}", index + 2, row.key);
        }
        if row.text.is_empty() {
            bail!("第 {} 条记录的 text 为空", index + 2);
        }
        if row.namespace == ASSET_TEXT_NAMESPACE
            && row.context_asset != TUTORIAL_MAP
            && !embedded_scope(&row.context_asset)
        {
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
    FixedFString,
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

fn scan_tutorial_strings(header: &[u8], data: &[u8]) -> Result<Vec<BlueprintMapString>> {
    let package = PackageLayout::read(header, data)?;
    let mut strings = Vec::new();
    for (index, export) in package.exports.iter().enumerate() {
        match package.object_name(package.export_classes[index]) {
            Some("Function") => {
                for mut item in scan_blueprint_map_strings(&data[export.start..export.end]) {
                    item.start += export.start;
                    let mut approved = false;
                    for call in item.start.saturating_sub(160).max(export.start)..item.start {
                        if data[call] != 0x68
                            || !package.import_is(
                                u32_at(data, call + 1)? as i32,
                                "KismetSystemLibrary",
                                "SetStringPropertyByName",
                            )
                        {
                            continue;
                        }
                        let Some(name_at) = package.simple_operand(data, call + 5) else {
                            continue;
                        };
                        if data.get(name_at) == Some(&0x21)
                            && package.fname(data, name_at + 1) == Some("DescText")
                            && name_at + 10 == item.start
                        {
                            let (_, end) = read_script_string(data, item.start - 1)
                                .context("字幕字符串无效")?;
                            approved = data.get(end) == Some(&0x16);
                        }
                    }
                    if approved {
                        strings.push(item);
                    }
                }
            }
            Some("LevelLocationMap_C") => {
                // This actor's display label is the first human-readable FString.
                // Actor names and localized FText sources are excluded. Keep the
                // narrow actor-class rule until other instance schemas are audited.
                if let Some(literal) = property_literals(data, export.start, export.end)
                    .into_iter()
                    .find(|literal| !literal.source.starts_with("LevelLocationMap"))
                {
                    let (_, end) = read_fstring_at(data, literal.offset).context("地图标签无效")?;
                    if (end - literal.offset - 4) % 2 == 0 {
                        strings.push(BlueprintMapString {
                            start: literal.offset,
                            text: literal.source,
                            encoding: BlueprintStringEncoding::FixedFString,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    strings.sort_by_key(|item| item.start);
    Ok(strings)
}

fn patch_tutorial_subtitles(
    source: &Path,
    header: &Path,
    target: &Path,
    rows: &[CsvRow],
) -> Result<usize> {
    let mut data = fs::read(source)?;
    let original_len = data.len();
    let strings = scan_tutorial_strings(&fs::read(header)?, &data)?;
    let mut occurrences = BTreeMap::<String, usize>::new();
    let map_rows: BTreeMap<&str, &CsvRow> = rows
        .iter()
        .filter(|row| row.namespace == ASSET_TEXT_NAMESPACE && row.context_asset == TUTORIAL_MAP)
        .map(|row| (row.key.as_str(), row))
        .collect();
    let mut matched = 0;
    for item in &strings {
        let ordinal = occurrences.entry(item.text.clone()).or_default();
        let key = asset_text_key(TUTORIAL_MAP, &item.text, *ordinal);
        *ordinal += 1;
        let Some(row) = map_rows.get(key.as_str()) else {
            continue;
        };
        matched += 1;
        if row.text != item.text || row.hash != unreal_text_crc(&item.text) {
            bail!(
                "教程地图 CSV 原文或哈希已失效：{}；请重新运行 decode",
                item.text
            );
        }
        replace_blueprint_string(&mut data, item, &row.trans_text)?;
    }
    if matched != map_rows.len() {
        bail!("教程地图部分 CSV 文本无法在当前结构中重新定位，请重新 decode");
    }
    if data.len() != original_len {
        bail!("教程地图等长替换校验失败");
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(target, data)
        .with_context(|| format!("无法写入教程地图文本补丁 {}", target.display()))?;
    Ok(matched)
}

fn replace_blueprint_string(
    data: &mut [u8],
    item: &BlueprintMapString,
    chinese: &str,
) -> Result<()> {
    let start = item.start;
    let english = &item.text;
    if start == 0 || start >= data.len() {
        bail!("教程地图文本偏移无效：{english}");
    }
    let slot_len = match item.encoding {
        BlueprintStringEncoding::FixedFString => {
            return replace_fixed_fstring(data, start, english, chinese);
        }
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
    // Loose encoding must leave untranslated literals intact, including ANSI
    // strings which would not fit if needlessly expanded to UTF-16.
    if chinese == english {
        return Ok(());
    }
    if chinese.contains('\0') {
        bail!("教程地图译文不能包含 NUL：{english}");
    }
    let odd_slot = slot_len % 2 != 0;
    if odd_slot {
        validate_tutorial_subtitle_call(data, start, slot_len)?;
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
    data[start - 1] = 0x34; // EX_UnicodeStringConst
    data[start..start + bytes.len()].copy_from_slice(&bytes);
    if odd_slot {
        // Use one fewer payload byte, move EndFunctionParms back by one, then
        // execute EX_Nothing after this void call. The next statement, script
        // size and all subsequent jump targets retain their original offsets.
        data[start + slot_len - 1] = 0x16; // EX_EndFunctionParms
        data[start + slot_len] = 0x0b; // EX_Nothing
    }
    Ok(())
}

fn replace_fixed_fstring(
    data: &mut [u8],
    start: usize,
    source: &str,
    translation: &str,
) -> Result<()> {
    let (original, end) = read_fstring_at(data, start).context("无效地图 FString")?;
    if original != source || translation.contains('\0') {
        bail!("地图 FString 原文或译文无效");
    }
    if translation == source {
        return Ok(());
    }
    let payload = end - start - 4;
    let mut units: Vec<u16> = translation.encode_utf16().collect();
    if payload % 2 != 0 || units.len() + 1 > payload / 2 {
        bail!("地图属性译文过长或槽位不支持等长 Unicode");
    }
    // Keep the serialized property and all subsequent actor/export offsets
    // unchanged. Spaces belong inside the FString, before its terminator.
    units.resize(payload / 2 - 1, ' ' as u16);
    units.push(0);
    data[start..start + 4].copy_from_slice(&(-(units.len() as i32)).to_le_bytes());
    for (slot, unit) in data[start + 4..end].chunks_exact_mut(2).zip(units) {
        slot.copy_from_slice(&unit.to_le_bytes());
    }
    let (verified, verified_end) = read_fstring_at(data, start).context("地图译文回读失败")?;
    if verified_end != end || verified.trim_end() != translation.trim_end() {
        bail!("地图译文等长回读校验失败");
    }
    Ok(())
}

fn validate_tutorial_subtitle_call(data: &[u8], start: usize, slot_len: usize) -> Result<()> {
    // Discovery resolves the function and DescText by name. Here validate the
    // supported standalone-call shape again, without pinning name/import IDs.
    let prefix = start.checked_sub(32).and_then(|at| data.get(at..start));
    let next = data.get(start + slot_len..start + slot_len + 19);
    let valid = match (prefix, next) {
        (Some(p), Some(n)) => {
            p[0] == 0x68
                && i32::from_le_bytes(p[1..5].try_into().unwrap()) < 0
                && matches!(p[5], 0x00 | 0x01 | 0x02 | 0x48)
                && p[6..10] == [1, 0, 0, 0]
                && p[22] == 0x21
                && p[27..31] == [0, 0, 0, 0]
                && p[31] == 0x1f
                && n[..2] == [0x16, 0x19]
                && p[5..22] == n[2..19]
        }
        _ => false,
    };
    if !valid {
        bail!("奇数字节字幕不在支持的独立属性赋值调用中");
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum UiLiteralLayout {
    StringAssignment { start: usize, end: usize },
    FString,
}

#[derive(Clone)]
struct UiLiteral {
    offset: usize,
    source: String,
    layout: UiLiteralLayout,
}

struct EmbeddedUiAsset {
    path: String,
    // A compatibility fingerprint, not a security boundary. These offsets and
    // statement boundaries were audited against V8.1.1's original assets.
    md5: String,
    header_md5: Option<String>,
    literals: Vec<UiLiteral>,
}

#[derive(Debug, Serialize)]
struct TextCandidate {
    asset: String,
    export: String,
    kind: String,
    offset: usize,
    end: usize,
    text: String,
    status: String,
    reason: String,
}

struct PackageLayout {
    names: Vec<String>,
    imports: Vec<(String, i32)>,
    export_names: Vec<String>,
    export_classes: Vec<i32>,
    exports: Vec<CookedExport>,
}

impl PackageLayout {
    fn read(header: &[u8], data: &[u8]) -> Result<Self> {
        let (exports, _) = cooked_item_exports(header, data)?;
        let summary = read_fstring_at(header, 52).context("包路径无效")?.1 + 4;
        let count = u32_at(header, summary)? as usize;
        let mut cursor = u32_at(header, summary + 4)? as usize;
        if count > header.len() / 8 {
            bail!("名称表长度无效");
        }
        let mut names = Vec::with_capacity(count);
        for _ in 0..count {
            let (name, end) = read_fstring_at(header, cursor).context("名称表损坏")?;
            cursor = end.checked_add(4).context("名称表溢出")?;
            if cursor > header.len() {
                bail!("名称表越界");
            }
            names.push(name);
        }
        let name_at = |at| -> Result<String> {
            let index = u32_at(header, at)? as usize;
            let number = u32_at(header, at + 4)?;
            let name = names.get(index).context("FName 索引无效")?;
            Ok(if number == 0 {
                name.clone()
            } else {
                format!("{name}_{}", number - 1)
            })
        };
        let import_count = u32_at(header, summary + 32)? as usize;
        let import_at = u32_at(header, summary + 36)? as usize;
        let export_at = u32_at(header, summary + 28)? as usize;
        if import_count > header.len() / 32
            || import_at.checked_add(import_count * 32) != Some(export_at)
        {
            bail!("不支持的导入表布局");
        }
        let mut imports = Vec::new();
        for i in 0..import_count {
            let at = import_at + i * 32;
            imports.push((name_at(at + 20)?, u32_at(header, at + 16)? as i32));
        }
        let mut export_names = Vec::new();
        let mut export_classes = Vec::new();
        for export in &exports {
            export_names.push(name_at(export.table_at + 16)?);
            export_classes.push(u32_at(header, export.table_at)? as i32);
        }
        Ok(Self {
            names,
            imports,
            export_names,
            export_classes,
            exports,
        })
    }

    fn object_name(&self, reference: i32) -> Option<&str> {
        if reference < 0 {
            self.imports
                .get(reference.unsigned_abs() as usize - 1)
                .map(|v| v.0.as_str())
        } else if reference > 0 {
            self.export_names
                .get(reference as usize - 1)
                .map(String::as_str)
        } else {
            None
        }
    }

    fn import_is(&self, reference: i32, owner: &str, function: &str) -> bool {
        reference < 0
            && self
                .imports
                .get(reference.unsigned_abs() as usize - 1)
                .is_some_and(|(name, outer)| {
                    name == function && self.object_name(*outer) == Some(owner)
                })
    }

    fn fname<'a>(&'a self, data: &[u8], at: usize) -> Option<&'a str> {
        if u32_at(data, at + 4).ok()? != 0 {
            return None;
        }
        self.names
            .get(u32_at(data, at).ok()? as usize)
            .map(String::as_str)
    }

    fn field_path(&self, data: &[u8], at: usize) -> Option<usize> {
        let count = u32_at(data, at).ok()? as usize;
        if count == 0 || count > 16 {
            return None;
        }
        for i in 0..count {
            self.names
                .get(u32_at(data, at + 4 + i * 8).ok()? as usize)?;
            u32_at(data, at + 8 + i * 8).ok()?;
        }
        let end = at + 4 + count * 8 + 4;
        self.object_name(u32_at(data, end - 4).ok()? as i32)?;
        Some(end)
    }

    fn simple_operand(&self, data: &[u8], at: usize) -> Option<usize> {
        match *data.get(at)? {
            0x00 | 0x01 | 0x02 | 0x48 => self.field_path(data, at + 1),
            0x1f | 0x34 => read_script_string(data, at).map(|(_, end)| end),
            0x25..=0x28 => Some(at + 1),
            _ => None,
        }
    }

    fn string_assignment(&self, data: &[u8], start: usize) -> Option<(usize, Vec<UiLiteral>)> {
        if *data.get(start)? != 0x0f {
            return None;
        }
        let target = self.field_path(data, start + 1)?;
        if !matches!(*data.get(target)?, 0x00 | 0x01 | 0x02 | 0x48) {
            return None;
        }
        let call = self.simple_operand(data, target)?;
        if *data.get(call)? != 0x68 {
            return None;
        }
        let function = u32_at(data, call + 1).ok()? as i32;
        let arguments = if self.import_is(function, "KismetStringLibrary", "Concat_StrStr") {
            2
        } else if self.import_is(function, "KismetMathLibrary", "SelectString") {
            3
        } else {
            return None;
        };
        let mut at = call + 5;
        let mut literals = Vec::new();
        for _ in 0..arguments {
            if let Some((source, _)) = read_script_string(data, at) {
                if candidate_text(&source) {
                    literals.push(UiLiteral {
                        offset: at,
                        source,
                        layout: UiLiteralLayout::StringAssignment { start, end: 0 },
                    });
                }
            }
            at = self.simple_operand(data, at)?;
        }
        if *data.get(at)? != 0x16 {
            return None;
        }
        let end = at + 1;
        for literal in &mut literals {
            literal.layout = UiLiteralLayout::StringAssignment { start, end };
        }
        Some((end, literals))
    }
}

fn candidate_text(text: &str) -> bool {
    let value = text.trim();
    (3..=4096).contains(&value.chars().count())
        && value.chars().any(|c| c.is_alphabetic())
        && !value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        && !value.contains('\0')
        && !value.starts_with('/')
        && !value.contains("\\")
        && !value.contains("/Game/")
        && !value.contains("/Engine/")
        && !value.contains('_')
        && !value.contains("://")
        && !(value.len() >= 16 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        && !(value.contains('.') && !value.contains(' ') && !value.ends_with("..."))
}

fn property_literals(data: &[u8], start: usize, end: usize) -> Vec<UiLiteral> {
    let mut localized_sources = HashSet::new();
    for at in start..end.saturating_sub(4) {
        if let Some((_, _, _)) = read_base_text_at_key(&data[..end], at) {
            if let Some((_, source_at)) = read_fstring_at(&data[..end], at) {
                localized_sources.insert(source_at);
            }
        }
    }
    let mut output = Vec::new();
    let mut at = start;
    while at + 4 <= end {
        let length = i32::from_le_bytes(data[at..at + 4].try_into().unwrap()).unsigned_abs();
        if (4..=4097).contains(&length) && !localized_sources.contains(&at) {
            if let Some((source, tail)) = read_fstring_at(&data[..end], at) {
                if candidate_text(&source) {
                    output.push(UiLiteral {
                        offset: at,
                        source,
                        layout: UiLiteralLayout::FString,
                    });
                    at = tail;
                    continue;
                }
            }
        }
        at += 1;
    }
    output
}

fn embedded_scope(path: &str) -> bool {
    path.ends_with(".uexp")
        && (is_ui_asset(path)
            || path.starts_with("NuclearNightmare/Content/Gameplay/Items/")
            || path.starts_with("NuclearNightmare/Content/Gameplay/Generators/"))
}

fn property_scope(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    (path.contains("/Gameplay/Items/") || path.contains("/Graphics/Computer/"))
        && !lower.contains("test")
        && !lower.contains("ammo")
}

fn discover_embedded_paths(pak_path: &Path) -> Result<Vec<String>> {
    let mut reader = BufReader::new(File::open(pak_path)?);
    let pak = repak::PakBuilder::new().reader(&mut reader)?;
    let files: HashSet<_> = pak.files().into_iter().collect();
    let mut paths: Vec<_> = files
        .iter()
        .filter(|path| embedded_scope(path) && files.contains(&path.replace(".uexp", ".uasset")))
        .cloned()
        .collect();
    paths.sort();
    Ok(paths)
}

fn scan_embedded_assets(
    extracted: &BTreeMap<String, PathBuf>,
    paths: &[String],
) -> Result<(Vec<EmbeddedUiAsset>, Vec<TextCandidate>)> {
    let mut assets = Vec::new();
    let mut report = Vec::new();
    for path in paths {
        let data = fs::read(extracted.get(path).context("缺少扫描资源")?)?;
        let header_path = path.replace(".uexp", ".uasset");
        let header = fs::read(extracted.get(&header_path).context("缺少扫描资源头")?)?;
        let package = match PackageLayout::read(&header, &data) {
            Ok(package) => package,
            Err(error) => {
                report.push(TextCandidate {
                    asset: path.clone(),
                    export: String::new(),
                    kind: "package".into(),
                    offset: 0,
                    end: data.len(),
                    text: String::new(),
                    status: "unsupported".into(),
                    reason: error.to_string(),
                });
                continue;
            }
        };
        let mut literals = Vec::new();
        for (index, export) in package.exports.iter().enumerate() {
            let mut found = Vec::new();
            let mut kind = "CDO FString";
            let allowed = property_scope(path);
            if export.is_default_object {
                found = property_literals(&data, export.start, export.end);
            } else if package.object_name(package.export_classes[index]) == Some("Function") {
                kind = "string assignment";
                let mut at = export.start;
                while at < export.end {
                    if let Some((end, group)) = package.string_assignment(&data[..export.end], at) {
                        found.extend(group);
                        at = end;
                    } else {
                        at += 1;
                    }
                }
            }
            for literal in found {
                let patchable = kind == "string assignment" || allowed;
                let end = match literal.layout {
                    UiLiteralLayout::FString => read_fstring_at(&data, literal.offset).unwrap().1,
                    UiLiteralLayout::StringAssignment { end, .. } => end,
                };
                report.push(TextCandidate {
                    asset: path.clone(),
                    export: package.export_names[index].clone(),
                    kind: kind.into(),
                    offset: literal.offset,
                    end,
                    text: literal.source.clone(),
                    status: if patchable {
                        "candidate"
                    } else {
                        "review_only"
                    }
                    .into(),
                    reason: if patchable {
                        "结构可回写；填写 CSV 译文后才纳入补丁"
                    } else {
                        "范围外属性（包括设置逻辑、测试物品及弹药默认值），仅报告"
                    }
                    .into(),
                });
                if patchable {
                    literals.push(literal);
                }
            }
        }
        literals.sort_by_key(|literal| literal.offset);
        if !literals.is_empty() {
            let properties = literals
                .iter()
                .any(|literal| matches!(literal.layout, UiLiteralLayout::FString));
            assets.push(EmbeddedUiAsset {
                path: path.clone(),
                md5: format!("{:X}", md5::compute(&data)),
                header_md5: properties.then(|| format!("{:X}", md5::compute(&header))),
                literals,
            });
        }
    }
    Ok((assets, report))
}

fn write_candidates(path: &Path, candidates: &[TextCandidate]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = BufWriter::new(File::create(path)?);
    file.write_all(&[0xef, 0xbb, 0xbf])?;
    let mut writer = csv::Writer::from_writer(file);
    for candidate in candidates {
        writer.serialize(candidate)?;
    }
    writer.flush()?;
    Ok(())
}

fn validate_discovered_rows(rows: &[CsvRow], assets: &[EmbeddedUiAsset]) -> Result<()> {
    let available: BTreeMap<_, _> = assets
        .iter()
        .flat_map(|asset| {
            asset
                .literals
                .iter()
                .enumerate()
                .map(move |(index, literal)| {
                    (
                        (asset.path.as_str(), embedded_ui_key(asset, index)),
                        literal.source.as_str(),
                    )
                })
        })
        .collect();
    for row in rows
        .iter()
        .filter(|row| row.namespace == ASSET_TEXT_NAMESPACE && row.context_asset != TUTORIAL_MAP)
    {
        if available
            .get(&(row.context_asset.as_str(), row.key.clone()))
            .copied()
            != Some(row.text.as_str())
            || row.hash != unreal_text_crc(&row.text)
        {
            bail!(
                "CSV 文本无法在当前支持结构中唯一定位：{} / {:?}；请重新 decode 并查看候选报告",
                row.context_asset,
                row.text
            );
        }
    }
    Ok(())
}

fn read_embedded_ui_asset(asset: &EmbeddedUiAsset, path: &Path) -> Result<Vec<u8>> {
    let data = fs::read(path)?;
    if format!("{:X}", md5::compute(&data)) != asset.md5 {
        bail!(
            "内嵌 UI 资源已变化，须重新核实文本和蓝图边界：{}",
            asset.path
        );
    }
    for literal in &asset.literals {
        let parsed = match literal.layout {
            UiLiteralLayout::StringAssignment { .. } => read_script_string(&data, literal.offset),
            UiLiteralLayout::FString => read_fstring_at(&data, literal.offset),
        };
        if parsed.as_ref().map(|(text, _)| text.as_str()) != Some(literal.source.as_str()) {
            bail!("内嵌 UI 原文不匹配：{} / {}", asset.path, literal.source);
        }
    }
    Ok(data)
}

fn embedded_ui_key(asset: &EmbeddedUiAsset, index: usize) -> String {
    let source = &asset.literals[index].source;
    let ordinal = asset.literals[..index]
        .iter()
        .filter(|literal| &literal.source == source)
        .count();
    asset_text_key(&asset.path, &source, ordinal)
}

fn embedded_ui_rows(
    extracted: &BTreeMap<String, PathBuf>,
    scanned_assets: &[EmbeddedUiAsset],
) -> Result<Vec<CsvRow>> {
    let mut rows = Vec::new();
    for asset in scanned_assets {
        read_embedded_ui_asset(
            asset,
            extracted.get(&asset.path).context("缺少内嵌 UI 资源")?,
        )?;
        if asset.header_md5.is_some() {
            read_item_ui_header(asset, extracted)?;
        }
        for (index, literal) in asset.literals.iter().enumerate() {
            rows.push(CsvRow {
                namespace: ASSET_TEXT_NAMESPACE.into(),
                key: embedded_ui_key(asset, index),
                hash: unreal_text_crc(&literal.source),
                text: literal.source.clone(),
                trans_text: String::new(),
                context_asset: asset.path.clone(),
            });
        }
    }
    Ok(rows)
}

fn patch_embedded_ui(
    extracted: &BTreeMap<String, PathBuf>,
    build: &Path,
    rows: &[CsvRow],
    scanned_assets: &[EmbeddedUiAsset],
) -> Result<()> {
    for asset in scanned_assets {
        let mut data = read_embedded_ui_asset(
            asset,
            extracted.get(&asset.path).context("缺少内嵌 UI 资源")?,
        )?;
        let asset_rows: Vec<_> = rows
            .iter()
            .filter(|row| row.namespace == ASSET_TEXT_NAMESPACE && row.context_asset == asset.path)
            .collect();
        if asset_rows.is_empty() {
            continue;
        }
        if asset_rows.len() > asset.literals.len() {
            bail!("内嵌 UI CSV 记录数不匹配，请重新 decode：{}", asset.path);
        }
        let translations: Vec<&str> = asset
            .literals
            .iter()
            .enumerate()
            .map(|(index, literal)| {
                let key = embedded_ui_key(asset, index);
                let row = asset_rows.iter().find(|row| row.key == key);
                let Some(row) = row else {
                    return Ok(literal.source.as_str());
                };
                if row.text != literal.source
                    || row.hash != unreal_text_crc(&literal.source)
                    || row.trans_text.contains('\0')
                {
                    bail!("内嵌 UI 原文、哈希或译文无效：{}", literal.source);
                }
                Ok(row.trans_text.as_str())
            })
            .collect::<Result<_>>()?;
        // Script edits keep their byte length. Apply them before resizing CDO
        // properties, which relocates later exports and their serialized data.
        let (script_literals, script_translations): (Vec<_>, Vec<_>) = asset
            .literals
            .iter()
            .zip(&translations)
            .filter(|(literal, _)| {
                matches!(literal.layout, UiLiteralLayout::StringAssignment { .. })
            })
            .map(|(literal, translation)| (literal.clone(), *translation))
            .unzip();
        replace_ui_assignments(&mut data, &script_literals, &script_translations)?;
        if asset.header_md5.is_some() {
            let mut header = read_item_ui_header(asset, extracted)?;
            let (properties, property_translations): (Vec<_>, Vec<_>) = asset
                .literals
                .iter()
                .zip(&translations)
                .filter(|(literal, _)| matches!(literal.layout, UiLiteralLayout::FString))
                .map(|(literal, translation)| (literal.clone(), *translation))
                .unzip();
            replace_item_properties(&mut header, &mut data, &properties, &property_translations)?;
            let target = build.join(asset.path.replace(".uexp", ".uasset"));
            fs::create_dir_all(target.parent().context("物品头文件目录无效")?)?;
            fs::write(target, header)?;
        } else if asset
            .literals
            .iter()
            .any(|literal| matches!(literal.layout, UiLiteralLayout::FString))
        {
            bail!("FString 属性缺少配套头文件：{}", asset.path);
        }
        let target = build.join(&asset.path);
        fs::create_dir_all(target.parent().context("UI 目标目录无效")?)?;
        fs::write(target, data)?;
    }
    Ok(())
}

fn read_item_ui_header(
    asset: &EmbeddedUiAsset,
    extracted: &BTreeMap<String, PathBuf>,
) -> Result<Vec<u8>> {
    let path = asset.path.replace(".uexp", ".uasset");
    let data = fs::read(extracted.get(&path).context("缺少物品 uasset 头文件")?)?;
    if Some(format!("{:X}", md5::compute(&data))) != asset.header_md5 {
        bail!("物品头文件已变化，须重新核实导出表：{path}");
    }
    Ok(data)
}

#[derive(Debug)]
struct CookedExport {
    table_at: usize,
    start: usize,
    end: usize,
    is_default_object: bool,
}

fn u32_at(data: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        data.get(at..at + 4)
            .context("资源 u32 字段越界")?
            .try_into()?,
    ))
}

fn i64_at(data: &[u8], at: usize) -> Result<i64> {
    Ok(i64::from_le_bytes(
        data.get(at..at + 8)
            .context("资源 i64 字段越界")?
            .try_into()?,
    ))
}

fn cooked_item_exports(header: &[u8], data: &[u8]) -> Result<(Vec<CookedExport>, usize)> {
    // Audited cooked UE5 layout, pinned by the .uasset/.uexp fingerprints.
    // Export map: class/super/template/outer (16), name (8), flags (4),
    // SerialSize (8), SerialOffset (8), then flags/dependencies; 96 bytes total.
    // These unversioned-property exports have no script-serialization ranges.
    if u32_at(header, 0)? != 0x9e2a83c1
        || u32_at(header, 4)? != (-9i32 as u32)
        || u32_at(header, 44)? as usize != header.len()
        || !data.ends_with(&0x9e2a83c1u32.to_le_bytes())
    {
        bail!("不支持的 cooked 物品资源头或尾标记");
    }
    let (_, folder_end) = read_fstring_at(header, 52).context("无效物品包路径")?;
    if u32_at(header, folder_end)? & 0x2000 == 0 {
        bail!("物品不是 unversioned-property 包");
    }
    let summary = folder_end + 4;
    let count = u32_at(header, summary + 24)? as usize;
    let table = u32_at(header, summary + 28)? as usize;
    let bulk_field = summary + 140;
    let expected_bulk = header.len() + data.len() - 4;
    if count == 0
        || count > header.len() / 96
        || table + count * 96 > header.len()
        || i64_at(header, bulk_field)? != expected_bulk as i64
    {
        bail!("物品导出表或 BulkDataStartOffset 不匹配");
    }
    let mut exports = Vec::new();
    let mut last = 0;
    for i in 0..count {
        let at = table + i * 96;
        let start = usize::try_from(i64_at(header, at + 36)? - header.len() as i64)?;
        let size = usize::try_from(i64_at(header, at + 28)?)?;
        let end = start.checked_add(size).context("物品导出长度溢出")?;
        if start != last || end > data.len() - 4 {
            bail!("物品导出表不连续或超出文件");
        }
        exports.push(CookedExport {
            table_at: at,
            start,
            end,
            is_default_object: u32_at(header, at + 24)? & 0x10 != 0,
        });
        last = end;
    }
    if last != data.len() - 4 {
        bail!("物品存在未识别的尾部数据");
    }
    Ok((exports, bulk_field))
}

fn replace_item_properties(
    header: &mut [u8],
    data: &mut Vec<u8>,
    literals: &[UiLiteral],
    translations: &[&str],
) -> Result<()> {
    if literals.len() != translations.len() || literals.is_empty() {
        bail!("物品译文数量不匹配");
    }
    let (exports, bulk_field) = cooked_item_exports(header, data)?;
    let mut edits = Vec::new();
    let mut owner = None;
    for (literal, translation) in literals.iter().zip(translations) {
        if !matches!(literal.layout, UiLiteralLayout::FString) || translation.contains('\0') {
            bail!("物品译文结构无效");
        }
        let (source, end) = read_fstring_at(data, literal.offset).context("物品 FString 无效")?;
        if source != literal.source {
            bail!("物品原文已变化：{}", literal.source);
        }
        let index = exports
            .iter()
            .position(|export| {
                export.is_default_object && literal.offset >= export.start && end <= export.end
            })
            .context("物品文本不在类默认对象中，不能改写")?;
        if owner.is_some_and(|previous| previous != index) {
            bail!("物品文本跨越多个默认对象");
        }
        owner = Some(index);
        let output = if *translation == source {
            data[literal.offset..end].to_vec()
        } else {
            let units: Vec<u16> = translation
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let length = i32::try_from(units.len())?;
            let mut output = (-length).to_le_bytes().to_vec();
            output.extend(units.into_iter().flat_map(u16::to_le_bytes));
            output
        };
        edits.push((literal.offset, end, output));
    }
    if edits.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        bail!("物品文本区域重叠或乱序");
    }
    let original = data.clone();
    let delta = |edit: &(usize, usize, Vec<u8>)| edit.2.len() as i64 - (edit.1 - edit.0) as i64;
    let total: i64 = edits.iter().map(delta).sum();
    for (start, end, bytes) in edits.iter().rev() {
        data.splice(*start..*end, bytes.iter().copied());
    }
    for export in &exports {
        let before: i64 = edits
            .iter()
            .filter(|edit| edit.1 <= export.start)
            .map(delta)
            .sum();
        let within: i64 = edits
            .iter()
            .filter(|edit| edit.0 >= export.start && edit.1 <= export.end)
            .map(delta)
            .sum();
        let size = (export.end - export.start) as i64 + within;
        let offset = header.len() as i64 + export.start as i64 + before;
        header[export.table_at + 28..export.table_at + 36].copy_from_slice(&size.to_le_bytes());
        header[export.table_at + 36..export.table_at + 44].copy_from_slice(&offset.to_le_bytes());
    }
    let bulk = i64_at(header, bulk_field)? + total;
    header[bulk_field..bulk_field + 8].copy_from_slice(&bulk.to_le_bytes());
    let (updated, _) = cooked_item_exports(header, data)?;
    for (index, (old, new)) in exports.iter().zip(updated).enumerate() {
        if Some(index) != owner && original[old.start..old.end] != data[new.start..new.end] {
            bail!("物品改写影响了其他导出（包括蓝图代码）");
        }
    }
    let mut shift = 0i64;
    for (edit, translation) in edits.iter().zip(translations) {
        let (actual, _) =
            read_fstring_at(data, (edit.0 as i64 + shift) as usize).context("物品译文回读失败")?;
        if actual != *translation {
            bail!("物品译文回读不一致");
        }
        shift += delta(edit);
    }
    Ok(())
}

fn replace_ui_assignments(
    data: &mut [u8],
    literals: &[UiLiteral],
    translations: &[&str],
) -> Result<()> {
    if literals.len() != translations.len() {
        bail!("UI 译文数量不匹配");
    }
    let mut groups = BTreeMap::<(usize, usize), Vec<(&UiLiteral, &str)>>::new();
    for (literal, translation) in literals.iter().zip(translations) {
        let UiLiteralLayout::StringAssignment { start, end } = literal.layout else {
            bail!("不是字符串赋值");
        };
        groups
            .entry((start, end))
            .or_default()
            .push((literal, translation));
    }
    let mut previous_end = 0;
    for ((start, end), mut group) in groups {
        if start < previous_end || start >= end || end > data.len() {
            bail!("UI 赋值边界重叠或越界");
        }
        previous_end = end;
        if group.len() == 1 {
            replace_ui_concat_literal(data, group[0].0, group[0].1)?;
            continue;
        }
        // MapWidget's EX_Let wraps SelectString (import -121): NIGHT, DAY,
        // then the unchanged boolean operand. Rewrite both literals together
        // so growth of DAY can consume the byte saved by NIGHT. Never insert
        // padding inside the nested expression or move the next statement.
        if data.get(start) != Some(&0x0f) || data.get(end - 1) != Some(&0x16) {
            bail!("UI 字符串选择赋值结构不匹配");
        }
        group.sort_by_key(|(literal, _)| literal.offset);
        let mut replacement = data[start..end].to_vec();
        let mut next = end;
        for (literal, translation) in group.into_iter().rev() {
            let (source, tail) =
                read_script_string(data, literal.offset).context("无效 UI 字符串")?;
            if literal.offset <= start
                || tail > next
                || source != literal.source
                || translation.contains('\0')
            {
                bail!("UI 字符串选择参数不匹配");
            }
            next = literal.offset;
            if translation == source {
                continue;
            }
            let bytes = std::iter::once(0x34).chain(
                translation
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .flat_map(u16::to_le_bytes),
            );
            replacement.splice(literal.offset - start..tail - start, bytes);
        }
        if replacement.len() > end - start {
            bail!("UI 字符串选择译文超过赋值槽位");
        }
        replacement.resize(end - start, 0x0b);
        data[start..end].copy_from_slice(&replacement);
    }
    Ok(())
}

fn replace_ui_concat_literal(
    data: &mut [u8],
    literal: &UiLiteral,
    translation: &str,
) -> Result<()> {
    let UiLiteralLayout::StringAssignment { start, end } = literal.layout else {
        bail!("不是字符串拼接赋值");
    };
    let (source, tail) = read_script_string(data, literal.offset).context("无效 UI 字符串")?;
    if source != literal.source
        || data.get(start) != Some(&0x0f)
        || data.get(end - 1) != Some(&0x16)
        || start >= literal.offset
        || tail > end
    {
        bail!("UI 拼接赋值结构不匹配");
    }
    if translation == source {
        return Ok(());
    }
    let bytes: Vec<u8> = translation
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let payload_start = literal.offset + 1;
    let old_len = tail - payload_start;
    if translation.contains('\0') || bytes.len() > old_len {
        bail!("内嵌 UI 译文包含 NUL 或过长：{translation}");
    }
    // The audited statements are EX_Let(..., Concat_StrStr(...)). There are no
    // jump/skip operands inside them. Keep the variable operand intact, move
    // the end of this assignment left, and pad AFTER it with EX_Nothing.
    // This avoids visible space padding between the prefix and dynamic name.
    let spare = old_len - bytes.len();
    data.copy_within(tail..end, tail - spare);
    data[literal.offset] = 0x34;
    data[payload_start..payload_start + bytes.len()].copy_from_slice(&bytes);
    data[end - spare..end].fill(0x0b);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_day_night_selection_preserves_boolean_operand_and_next_statement() -> Result<()> {
        let hex = "0F0100000040000000000000004700000000010000004000000000000000470000006887FFFFFF1F4E4947485420001F4441592000000100000018000000000000004700000016";
        let original: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
            .collect();
        let end = original.len();
        let literals = [
            UiLiteral {
                offset: 39,
                source: "NIGHT ".into(),
                layout: UiLiteralLayout::StringAssignment { start: 0, end },
            },
            UiLiteral {
                offset: 47,
                source: "DAY ".into(),
                layout: UiLiteralLayout::StringAssignment { start: 0, end },
            },
        ];
        let mut data = original.clone();
        data.extend_from_slice(&[0x0f, 0xaa, 0xbb]);
        let tail = data[53..].to_vec();
        replace_ui_assignments(&mut data, &literals, &["夜 ", "日 "])?;
        assert_eq!(read_script_string(&data, 39), Some(("夜 ".into(), 46)));
        assert_eq!(read_script_string(&data, 46), Some(("日 ".into(), 53)));
        assert_eq!(&data[53..], tail);
        assert_eq!(&data[..39], &original[..39]);
        let mut fallback = original.clone();
        replace_ui_assignments(&mut fallback, &literals, &["NIGHT ", "DAY "])?;
        assert_eq!(fallback, original);
        let mut rejected = original.clone();
        assert!(
            replace_ui_assignments(&mut rejected, &literals, &["夜晚时段", "白天时段"]).is_err()
        );
        assert_eq!(rejected, original);
        Ok(())
    }

    #[test]
    fn map_location_fstring_preserves_adjacent_actor_data() -> Result<()> {
        let mut original = vec![0xaa, 0xbb];
        fstring(&mut original, "Helicopter Location");
        original.extend_from_slice(&[0x2e, 0x32, 0, 0, 1, 0, 0, 0]);
        let mut data = original.clone();
        replace_fixed_fstring(&mut data, 2, "Helicopter Location", "直升机位置")?;
        let (text, end) = read_fstring_at(&data, 2).unwrap();
        assert_eq!(text.trim_end(), "直升机位置");
        assert_eq!(end, 26);
        assert_eq!(data.len(), original.len());
        assert_eq!(&data[..2], &original[..2]);
        assert_eq!(&data[end..], &original[end..]);
        for translation in ["超过当前属性预留的长度上限", "不含\0空字符"] {
            let mut rejected = original.clone();
            assert!(
                replace_fixed_fstring(&mut rejected, 2, "Helicopter Location", translation)
                    .is_err()
            );
            assert_eq!(rejected, original);
        }
        let mut fallback = original.clone();
        replace_fixed_fstring(
            &mut fallback,
            2,
            "Helicopter Location",
            "Helicopter Location",
        )?;
        assert_eq!(fallback, original);
        Ok(())
    }

    #[test]
    fn item_property_resizing_relocates_exports_and_preserves_gameplay_data() -> Result<()> {
        let mut header = vec![0; 52];
        fstring(&mut header, "/Game/TestItem");
        let flags_at = header.len();
        header.extend_from_slice(&0x2000u32.to_le_bytes());
        let summary = header.len();
        header.resize(768, 0);
        for (at, value) in [
            (0, 0x9e2a83c1),
            (4, -9i32 as u32),
            (44, 768),
            (summary + 24, 3),
            (summary + 28, 320),
        ] {
            header[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        let mut data = vec![0x53, 0xaa, 0xbb, 0xcc, 0x11, 0x22];
        let name_at = data.len();
        fstring(&mut data, "MP5K");
        let gap = [0x33; 24];
        data.extend_from_slice(&gap);
        let hint_at = data.len();
        fstring(&mut data, "(LMB) to shoot");
        data.extend_from_slice(&[0x44, 0x55]);
        let cdo_end = data.len();
        data.extend_from_slice(&[0x66, 0x77, 0x88]);
        let data_end = data.len();
        data.extend_from_slice(&0x9e2a83c1u32.to_le_bytes());
        for (i, (start, end, flags)) in [(0, 4, 1u32), (4, cdo_end, 0x39), (cdo_end, data_end, 8)]
            .into_iter()
            .enumerate()
        {
            let at = 320 + 96 * i;
            header[at + 24..at + 28].copy_from_slice(&flags.to_le_bytes());
            header[at + 28..at + 36].copy_from_slice(&((end - start) as i64).to_le_bytes());
            header[at + 36..at + 44].copy_from_slice(&(768i64 + start as i64).to_le_bytes());
        }
        header[summary + 140..summary + 148]
            .copy_from_slice(&(768i64 + data_end as i64).to_le_bytes());
        let original = data.clone();
        let original_header = header.clone();
        let literals = [
            UiLiteral {
                offset: name_at,
                source: "MP5K".into(),
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: hint_at,
                source: "(LMB) to shoot".into(),
                layout: UiLiteralLayout::FString,
            },
        ];
        for hint in [
            "(LMB) 射击",
            "按住 (LMB) 开火，按 (RMB) 瞄准",
            "开火",
            "(LMB) to shoot",
        ] {
            let mut current = original.clone();
            let mut current_header = original_header.clone();
            replace_item_properties(
                &mut current_header,
                &mut current,
                &literals,
                &["MP5K", hint],
            )?;
            let (_, next) = read_fstring_at(&current, name_at).unwrap();
            assert_eq!(&current[next..next + 24], &gap);
            assert_eq!(read_fstring_at(&current, next + 24).unwrap().0, hint);
            let (exports, _) = cooked_item_exports(&current_header, &current)?;
            assert_eq!(&current[..4], &original[..4]);
            assert_eq!(&current[exports[2].start..], &original[cdo_end..]);
            assert_eq!(exports[1].end, exports[2].start);
            if hint == "(LMB) to shoot" {
                assert_eq!(current, original);
                assert_eq!(current_header, original_header);
            }
        }
        // A changed header or a non-CDO target must fail before mutating bytes.
        for at in [flags_at, 320 + 96 + 24] {
            let mut changed_header = original_header.clone();
            changed_header[at..at + 4].fill(0);
            let mut changed_data = original.clone();
            assert!(
                replace_item_properties(
                    &mut changed_header,
                    &mut changed_data,
                    &literals,
                    &["MP5K", "开火"]
                )
                .is_err()
            );
            assert_eq!(changed_data, original);
        }
        Ok(())
    }

    #[test]
    fn embedded_ui_scope_excludes_unrelated_packages() {
        assert!(embedded_scope(
            "NuclearNightmare/Content/Graphics/Computer/NewTerminal.uexp"
        ));
        assert!(property_scope(
            "NuclearNightmare/Content/Gameplay/Items/Item_NewTool.uexp"
        ));
        assert!(!property_scope(
            "NuclearNightmare/Content/Gameplay/Items/Item_Ammo.uexp"
        ));
        assert!(!property_scope(
            "NuclearNightmare/Content/Graphics/MainMenu/SettingsMenu.uexp"
        ));
        assert!(!embedded_scope("NuclearNightmare/Content/Unknown.uexp"));
    }

    #[test]
    fn concat_ui_preserves_dynamic_location_and_statement_offsets() -> Result<()> {
        // Original generator notification assignment, including the location
        // variable operand following the literal. Keep this fixture independent
        // of the replacement implementation and the audited asset table.
        let hex = "0F0100000021000000000000000B000000000100000021000000000000000B000000687EFFFFFF1F2068617320706F7765726564206F6E20746865200001010000009F000000000000000300000016";
        let mut data: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
            .collect();
        let end = data.len();
        data.extend_from_slice(&[0x53, 0xaa]);
        let original = data.clone();
        let literal = UiLiteral {
            offset: 39,
            source: " has powered on the ".into(),
            layout: UiLiteralLayout::StringAssignment { start: 0, end },
        };
        replace_ui_concat_literal(&mut data, &literal, " 已启动 ")?;
        let (text, tail) = read_script_string(&data, 39).unwrap();
        assert_eq!(text, " 已启动 ");
        assert_eq!(&data[tail..tail + 18], &original[end - 18..end]);
        assert!(data[tail + 18..end].iter().all(|byte| *byte == 0x0b));
        assert_eq!(&data[..39], &original[..39]);
        assert_eq!(&data[end..], &[0x53, 0xaa]);
        assert_eq!(data.len(), original.len());
        let mut too_long = original.clone();
        assert!(replace_ui_concat_literal(&mut too_long, &literal, &"字".repeat(30)).is_err());
        assert_eq!(too_long, original);
        Ok(())
    }

    #[test]
    fn inventory_drop_prefix_fits_and_keeps_the_item_name_operand() -> Result<()> {
        let hex = "0F0100000013000000000000000F000000000100000013000000000000000F0000006890FFFFFF1F28512920546F2044726F702000010100000087000000000000009A00000016";
        let mut data: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
            .collect();
        let original = data.clone();
        let end = data.len();
        let literal = UiLiteral {
            offset: 39,
            source: "(Q) To Drop ".into(),
            layout: UiLiteralLayout::StringAssignment { start: 0, end },
        };
        replace_ui_concat_literal(&mut data, &literal, "(Q)丢弃")?;
        let (text, tail) = read_script_string(&data, 39).unwrap();
        assert_eq!(text, "(Q)丢弃");
        assert_eq!(&data[tail..tail + 18], &original[end - 18..end]);
        assert_eq!(data[end - 1], 0x0b);
        assert_eq!(data.len(), original.len());
        Ok(())
    }

    fn evacuation_subtitle_fixture() -> (Vec<u8>, BlueprintMapString) {
        // Captured call boundaries from the original V8.1.1 TutorialMap.
        let text = "Generators also power evacuation computers. Once at least 4 classified materials have been collected, you can use an evacuation computer to call for extraction and win the game or revive dead players.";
        let mut data = vec![
            0x68, 0x3c, 0xff, 0xff, 0xff, 0, 1, 0, 0, 0, 0x5d, 0, 0, 0, 6, 0, 0, 0, 0x4b, 0, 0, 0,
            0x21, 0x8c, 0, 0, 0, 0, 0, 0, 0, 0x1f,
        ];
        let start = data.len();
        data.extend_from_slice(text.as_bytes());
        data.extend_from_slice(&[
            0, 0x16, 0x19, 0, 1, 0, 0, 0, 0x5d, 0, 0, 0, 6, 0, 0, 0, 0x4b, 0, 0, 0,
        ]);
        (
            data,
            BlueprintMapString {
                start,
                text: text.into(),
                encoding: BlueprintStringEncoding::Ansi,
            },
        )
    }

    #[test]
    fn evacuation_subtitle_is_unicode_and_preserves_next_statement() -> Result<()> {
        let (mut data, item) = evacuation_subtitle_fixture();
        let original = data.clone();
        let translation = "发电机也会为撤离电脑供电。收集至少 4 份机密材料后，你可以使用撤离电脑呼叫撤离并赢得游戏，或复活已死亡的玩家。";
        replace_blueprint_string(&mut data, &item, translation)?;
        let (decoded, next) = read_script_string(&data, item.start - 1).unwrap();
        assert_eq!(decoded.trim_end(), translation);
        assert_eq!(data[item.start - 1], 0x34);
        assert_eq!(next, item.start + item.text.len());
        assert_eq!(&data[next..next + 2], &[0x16, 0x0b]);
        assert_eq!(&data[next + 2..], &original[next + 2..]);
        assert_eq!(&data[..item.start - 1], &original[..item.start - 1]);
        assert_eq!(data.len(), original.len());
        Ok(())
    }

    #[test]
    fn rejects_unaudited_odd_slots_without_modifying_data() {
        for change in [0, 22, 233] {
            let (mut data, item) = evacuation_subtitle_fixture();
            data[change] ^= 1;
            let original = data.clone();
            assert!(replace_blueprint_string(&mut data, &item, "测试字幕").is_err());
            assert_eq!(data, original);
        }
        let (mut data, item) = evacuation_subtitle_fixture();
        let original = data.clone();
        assert!(replace_blueprint_string(&mut data, &item, &"字".repeat(100)).is_err());
        assert_eq!(data, original);
    }

    #[test]
    fn untranslated_ansi_and_even_unicode_slots_remain_valid() -> Result<()> {
        let (mut data, item) = evacuation_subtitle_fixture();
        let original = data.clone();
        replace_blueprint_string(&mut data, &item, &item.text)?;
        assert_eq!(data, original);
        let mut data = b"\x1fAn example text\0\x16\x53".to_vec();
        let item = BlueprintMapString {
            start: 1,
            text: "An example text".into(),
            encoding: BlueprintStringEncoding::Ansi,
        };
        replace_blueprint_string(&mut data, &item, "测试字幕")?;
        let (decoded, next) = read_script_string(&data, 0).unwrap();
        assert_eq!(decoded.trim_end(), "测试字幕");
        assert_eq!(&data[next..], &[0x16, 0x53]);
        let item = BlueprintMapString {
            start: 1,
            text: decoded,
            encoding: BlueprintStringEncoding::Unicode,
        };
        replace_blueprint_string(&mut data, &item, "中文🎮")?;
        let (decoded, next) = read_script_string(&data, 0).unwrap();
        assert_eq!(decoded.trim_end(), "中文🎮");
        assert_eq!(&data[next..], &[0x16, 0x53]);
        Ok(())
    }

    #[test]
    fn font_override_wraps_sarasa_and_rejects_changed_container() -> Result<()> {
        let template = [4, 0, 0, 0, b'O', b'T', b'T', b'O', 0, 0, 0, 0];
        let wrapped = build_font_override(&template, SARASA_FONT)?;
        assert_eq!(
            i32::from_le_bytes(wrapped[..4].try_into()?) as usize,
            SARASA_FONT.len()
        );
        assert_eq!(&wrapped[4..wrapped.len() - 4], SARASA_FONT);
        assert_eq!(&wrapped[wrapped.len() - 4..], &[0; 4]);
        assert!(build_font_override(&[], SARASA_FONT).is_err());
        assert!(build_font_override(&template[..8], SARASA_FONT).is_err());
        let mut changed = template;
        changed[11] = 1;
        assert!(build_font_override(&changed, SARASA_FONT).is_err());
        assert!(build_font_override(&template, b"invalid font").is_err());
        Ok(())
    }

    #[test]
    fn pak_roundtrip_checks_compressed_blocks_and_plain_entries() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let build = dir.path().join("build");
        fs::create_dir_all(build.join("Content"))?;
        // Cross multiple compression blocks, including a partial final block.
        let payload: Vec<u8> = (0..300_001).map(|i| (i % 251) as u8).collect();
        fs::write(build.join("Content/payload.bin"), &payload)?;
        fs::write(build.join("Content/empty.bin"), [])?;
        let entries = collect_build_entries(&build)?;
        let metadata = PakMetadata {
            version: repak::Version::V11,
            mount_point: "../../../".into(),
            path_hash_seed: Some(1111855876),
        };
        for compress in [false, true] {
            let output = dir.path().join(format!("{compress}.pak"));
            write_pak(&output, &build, &entries, &metadata, compress)?;
            verify_pak(&output, &entries, &metadata, &build)?;
            let mut reader = BufReader::new(File::open(&output)?);
            let pak = repak::PakBuilder::new().reader(&mut reader)?;
            assert_eq!(
                pak.used_compression(),
                if compress {
                    vec![repak::Compression::Zlib]
                } else {
                    vec![]
                }
            );
            if compress {
                assert!(fs::metadata(&output)?.len() < payload.len() as u64);
            }
            let mut changed = payload.clone();
            changed[150_000] ^= 1;
            fs::write(build.join("Content/payload.bin"), changed)?;
            assert!(verify_pak(&output, &entries, &metadata, &build).is_err());
            fs::write(build.join("Content/payload.bin"), &payload)?;
        }
        Ok(())
    }

    fn fstring(out: &mut Vec<u8>, text: &str) {
        out.extend_from_slice(&((text.len() + 1) as i32).to_le_bytes());
        out.extend_from_slice(text.as_bytes());
        out.push(0);
    }

    fn base(namespace: &str, key: &str, text: &str) -> Vec<u8> {
        let mut out = vec![0; 5];
        fstring(&mut out, namespace);
        fstring(&mut out, key);
        fstring(&mut out, text);
        out
    }

    fn scan(data: &[u8]) -> Result<LiveTexts> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("fixture.uexp");
        fs::write(&path, data)?;
        scan_live_ftext(&[("fixture".into(), path)])
    }

    #[test]
    fn preserves_named_keys_and_separate_namespaces() -> Result<()> {
        let mut data = base("NNUIRemaster", "Screen_Heading", "FIELD TERMINAL");
        data.extend(base("Other", "Screen_Heading", "OTHER TITLE"));
        data.extend(base("", "ABCDEF0123456789ABCDEF0123456789", "Legacy"));
        data.extend(base("", "01234567890123456789012345678901", "Numeric GUID"));
        let live = scan(&data)?;
        assert_eq!(live.len(), 4);
        assert_eq!(
            live[&("NNUIRemaster".into(), "Screen_Heading".into())].0,
            "FIELD TERMINAL"
        );
        Ok(())
    }

    #[test]
    fn reads_runtime_template_and_unicode_literals() -> Result<()> {
        let mut data = vec![0x29, 1];
        for text in [
            "PING {M} ms ({State})",
            "C420C09549DF6364CCE3F3909A777130",
            "",
        ] {
            data.push(0x1f);
            data.extend(text.as_bytes());
            data.push(0);
        }
        data.extend([0x29, 1, 0x34]);
        for unit in "温度 {T}".encode_utf16().chain(std::iter::once(0)) {
            data.extend(unit.to_le_bytes());
        }
        for text in ["Temperature", "Runtime"] {
            data.push(0x1f);
            data.extend(text.as_bytes());
            data.push(0);
        }
        let live = scan(&data)?;
        assert_eq!(live.len(), 2);
        assert_eq!(
            live[&("Runtime".into(), "Temperature".into())].0,
            "温度 {T}"
        );
        assert_eq!(
            live[&("".into(), "C420C09549DF6364CCE3F3909A777130".into())].0,
            "PING {M} ms ({State})"
        );
        Ok(())
    }

    #[test]
    fn rejects_conflicting_sources_and_truncated_data() -> Result<()> {
        let mut data = base("UI", "Title", "One");
        data.extend(base("UI", "Title", "Two"));
        assert!(scan(&data).is_err());
        let mut truncated = base("UI", "Title", "One");
        truncated.pop();
        assert!(scan(&truncated)?.is_empty());
        assert_eq!(read_fstring_at(&i32::MIN.to_le_bytes(), 0), None);
        assert_eq!(read_script_string(&[0x34, 0x41], 0), None);
        Ok(())
    }

    #[test]
    fn changed_source_does_not_inherit_translation() -> Result<()> {
        let old = CsvRow {
            namespace: "UI".into(),
            key: "Title".into(),
            hash: unreal_text_crc("Old"),
            text: "Old".into(),
            trans_text: "旧".into(),
            context_asset: "".into(),
        };
        let mut current = vec![CsvRow {
            text: "New".into(),
            hash: unreal_text_crc("New"),
            trans_text: "".into(),
            ..old.clone()
        }];
        assert_eq!(merge_translations(&mut current, vec![old]), 0);
        assert!(current[0].trans_text.is_empty());
        current[0].trans_text = "新".into();
        validate_rows(&current, true)?;
        current[0].text = "PING {M} ms ({State})".into();
        assert!(fill_and_validate_translations(&mut current).is_err());
        current[0].text = "{M} / {M}".into();
        current[0].trans_text = "{M}".into();
        assert!(fill_and_validate_translations(&mut current).is_err());
        current[0].trans_text = "{M}，{M}".into();
        fill_and_validate_translations(&mut current)?;
        Ok(())
    }
}

fn main()->Result<()> {
 let rows=read_csv(Path::new("translations.csv"))?;
 let mut paths:Vec<_>=rows.iter().filter(|r|r.namespace==ASSET_TEXT_NAMESPACE && r.context_asset!=TUTORIAL_MAP).map(|r|r.context_asset.clone()).collect();paths.sort();paths.dedup();
 let mut files=BTreeMap::new();for p in &paths {for p in [p.clone(),p.replace(".uexp",".uasset")] {let leaf=Path::new(&p).file_name().unwrap().to_string_lossy();let local=PathBuf::from("target/computer-ui/encode").join(format!("{:08x}-{leaf}",unreal_text_crc(&p)));if !local.exists(){let old=PathBuf::from("target/map-ui").join(format!("{:08x}-{leaf}",unreal_text_crc(&p)));if old.exists(){files.insert(p,old);continue;}let old=PathBuf::from("target/ui-missing").join(format!("{:08x}-{leaf}",unreal_text_crc(&p)));if old.exists(){files.insert(p,old);continue;}bail!("Missing {local:?}");}files.insert(p,local);}}
 let (assets,report)=scan_embedded_assets(&files,&paths)?;write_candidates(Path::new("target/auto-discovery/baseline-candidates.csv"),&report)?;
 println!("Scanned {} assets, {} candidates; {} eligible literals",assets.len(),report.len(),assets.iter().map(|a|a.literals.len()).sum::<usize>());
 validate_discovered_rows(&rows,&assets)?;
 let h=fs::read("target/update-20261024/045f4c22-TutorialMap.umap")?;let d=fs::read("target/update-20261024/decode/61a710dc-TutorialMap.uexp")?;
 for v in scan_tutorial_strings(&h,&d)? {println!("MAP {} {:?}",v.start,v.text);}
 println!("All existing UI CSV keys rediscovered without pinned offsets/hashes.");Ok(())
}