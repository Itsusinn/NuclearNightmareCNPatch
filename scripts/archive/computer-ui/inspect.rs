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
    for asset in EMBEDDED_UI_ASSETS {
        if !wanted.iter().any(|path| path == asset.path) {
            wanted.push(asset.path.to_string());
        }
        if asset.header_md5.is_some() {
            wanted.push(asset.path.replace(".uexp", ".uasset"));
        }
    }
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
    let tutorial_strings = scan_tutorial_strings(&tutorial_data)?;
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

    rows.extend(embedded_ui_rows(&extracted)?);
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
    encode_entries.extend(
        EMBEDDED_UI_ASSETS
            .iter()
            .map(|asset| asset.path.to_string()),
    );
    encode_entries.extend(
        EMBEDDED_UI_ASSETS
            .iter()
            .filter(|asset| asset.header_md5.is_some())
            .map(|asset| asset.path.replace(".uexp", ".uasset")),
    );
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

    patch_embedded_ui(&extracted, &build, &rows)?;
    let tutorial_source = extracted
        .get(TUTORIAL_MAP)
        .context("内部错误：没有提取教程地图字节码")?;
    let tutorial_target = build.join(TUTORIAL_MAP);
    let tutorial_patch_count = patch_tutorial_subtitles(tutorial_source, &tutorial_target, &rows)?;

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
        EMBEDDED_UI_ASSETS.len(),
        EMBEDDED_UI_ASSETS
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
            && !EMBEDDED_UI_ASSETS
                .iter()
                .any(|asset| asset.path == row.context_asset)
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

fn scan_tutorial_strings(data: &[u8]) -> Result<Vec<BlueprintMapString>> {
    // The placed LevelLocationMap actor overrides the already-localized widget
    // label with this FString. Its property slot is separate from script code.
    const OFFSET: usize = 124658455;
    const LABEL: &str = "Helicopter Location";
    if format!("{:X}", md5::compute(data)) != "AF9AF47FFFDCB3042826A449E807DDA7"
        || read_fstring_at(data, OFFSET)
            .as_ref()
            .map(|(s, _)| s.as_str())
            != Some(LABEL)
    {
        bail!("教程地图已变化，须重新核实直升机标记属性位置");
    }
    let mut strings = scan_blueprint_map_strings(data);
    strings.push(BlueprintMapString {
        start: OFFSET,
        text: LABEL.into(),
        encoding: BlueprintStringEncoding::FixedFString,
    });
    Ok(strings)
}

fn patch_tutorial_subtitles(source: &Path, target: &Path, rows: &[CsvRow]) -> Result<usize> {
    let mut data = fs::read(source)?;
    let original_len = data.len();
    let strings = scan_tutorial_strings(&data)?;
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
    // Verified against TutorialMap V8.1.1's import/name tables: import -196 is
    // KismetSystemLibrary.SetStringPropertyByName (void), name 140 is DescText,
    // and name 93 is CallFunc_Create_ReturnValue. This is a standalone call,
    // with the literal as its final parameter, followed by an EX_Context call.
    // Do NOT apply the padding trick to arbitrary nested string expressions.
    // Fail closed when a game update changes this audited bytecode layout.
    const CALL: &[u8] = &[
        0x68, 0x3c, 0xff, 0xff, 0xff, // EX_CallMath, import -196
        0x00, 1, 0, 0, 0, 0x5d, 0, 0, 0, 6, 0, 0, 0, 0x4b, 0, 0, 0, 0x21, 0x8c, 0, 0, 0, 0, 0, 0,
        0,    // EX_NameConst, DescText
        0x1f, // EX_StringConst
    ];
    const NEXT: &[u8] = &[
        0x16, // EX_EndFunctionParms
        0x19, 0x00, 1, 0, 0, 0, 0x5d, 0, 0, 0, 6, 0, 0, 0, 0x4b, 0, 0, 0,
    ];
    let prefix = start
        .checked_sub(CALL.len())
        .and_then(|at| data.get(at..start));
    if prefix != Some(CALL)
        || data.get(start + slot_len..start + slot_len + NEXT.len()) != Some(NEXT)
    {
        bail!(
            "教程地图奇数字节字符串不在已核实的独立字幕调用中，不能安全转换为 Unicode；请检查当前游戏蓝图"
        );
    }
    Ok(())
}

enum UiLiteralLayout {
    StringAssignment { start: usize, end: usize },
    FString,
}

struct UiLiteral {
    offset: usize,
    source: &'static str,
    layout: UiLiteralLayout,
}

struct EmbeddedUiAsset {
    path: &'static str,
    // A compatibility fingerprint, not a security boundary. These offsets and
    // statement boundaries were audited against V8.1.1's original assets.
    md5: &'static str,
    header_md5: Option<&'static str>,
    literals: &'static [UiLiteral],
}

const EMBEDDED_UI_ASSETS: &[EmbeddedUiAsset] = &[
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Graphics/Journal/MapWidget.uexp",
        md5: "804FFC1831C948B064E5FDD8C3E718D8",
        header_md5: None,
        literals: &[
            UiLiteral {
                offset: 40925,
                source: "NIGHT ",
                layout: UiLiteralLayout::StringAssignment {
                    start: 40886,
                    end: 40957,
                },
            },
            UiLiteral {
                offset: 40933,
                source: "DAY ",
                layout: UiLiteralLayout::StringAssignment {
                    start: 40886,
                    end: 40957,
                },
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Generators/BP_GeneratorGameplayNew.uexp",
        md5: "E60F752D8DE2D4BDE7788E930F672211",
        header_md5: None,
        literals: &[
            UiLiteral {
                offset: 30267,
                source: " has powered on the ",
                layout: UiLiteralLayout::StringAssignment {
                    start: 30228,
                    end: 30307,
                },
            },
            UiLiteral {
                offset: 30363,
                source: " generator...",
                layout: UiLiteralLayout::StringAssignment {
                    start: 30307,
                    end: 30379,
                },
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Graphics/Inventory/InventoryUI.uexp",
        md5: "EDB4D9C47FAA419F93AF9D382CFA62E5",
        header_md5: None,
        literals: &[UiLiteral {
            offset: 18136,
            source: "(Q) To Drop ",
            layout: UiLiteralLayout::StringAssignment {
                start: 18097,
                end: 18168,
            },
        }],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Barricade.uexp",
        md5: "B02306FCE7D691C61B88D73DD24488F4",
        header_md5: Some("6A0D3F7C99D28D243F922FF65B0A83C5"),
        literals: &[
            UiLiteral {
                offset: 4611,
                source: "Barricade",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 4625,
                source: "(E) Interact with a door when closed to use.",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_BeefJerky.uexp",
        md5: "1AE76F81B130254B72B927C8717002A4",
        header_md5: Some("FA6E4A0C527B45EFE239FEA1147D9102"),
        literals: &[
            UiLiteral {
                offset: 4607,
                source: "Beef Jerky",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 4646,
                source: "Used for feeding huskies...",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Cassette.uexp",
        md5: "906FE470745F9EA6FBB1683C5F1C6FEA",
        header_md5: Some("4151F449D4D3D0DB2D023A1915BDAD4B"),
        literals: &[
            UiLiteral {
                offset: 4668,
                source: "Cassette",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 4705,
                source: "(E) interact with tape recorder to play",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Flamethrower.uexp",
        md5: "5519A37AF75548152C0043B1A00595B4",
        header_md5: Some("76B8D2514397FC7AFADF5EAAC08F1B3A"),
        literals: &[
            UiLiteral {
                offset: 5248,
                source: "Flamethrower",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 5289,
                source: "(LMB) to Use Flamethrower",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Flare.uexp",
        md5: "69E18C054999A444339EF421F79AB317",
        header_md5: Some("5A5FE467DBFE919B593572C857B2B2E9"),
        literals: &[
            UiLiteral {
                offset: 4691,
                source: "Flare",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 4701,
                source: "Press (LMB) to use Flare",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_FlareGun.uexp",
        md5: "E55E77B4C786A3AB1D679C0863376C7E",
        header_md5: Some("1589420B7CC955D030CD5223958B3C60"),
        literals: &[
            UiLiteral {
                offset: 4668,
                source: "Flare Gun",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 4706,
                source: "(LMB) to shoot",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Flashlight.uexp",
        md5: "A23AD7C214FDEA9B63FE16F382D79308",
        header_md5: Some("0375868095C14B833F31D6A5710806A0"),
        literals: &[
            UiLiteral {
                offset: 5832,
                source: "Flashlight",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 5871,
                source: "(F) To Toggle Flashlight",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_GasCanster.uexp",
        md5: "9E8B02589B8159FEBFA46641DDB256CB",
        header_md5: Some("7480276A150404FE4B88C4D47B200AC1"),
        literals: &[
            UiLiteral {
                offset: 3889,
                source: "Gas Canister",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 3906,
                source: "(LMB) to Refuel Snowmobile",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Glowstick.uexp",
        md5: "07519997E50A58A6B624783BDDD45A5B",
        header_md5: Some("4662488A09351E346886459FF3A9904F"),
        literals: &[
            UiLiteral {
                offset: 4601,
                source: "Glowstick",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 4615,
                source: "(G) To Toggle Glowstick",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Grenade.uexp",
        md5: "EE96AABEA70BA3787EDF4A888C3C2AF2",
        header_md5: Some("4FF69FA0AA1C84B61CAE4FB5A843595A"),
        literals: &[
            UiLiteral {
                offset: 4670,
                source: "Grenade",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 4706,
                source: "(LMB) to once to aim then to throw...",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_MotionTracker.uexp",
        md5: "69F84FF6AC2F13216EE1541F6DC66F25",
        header_md5: Some("3F94A7A3CDFD87B72CB8902C013DAD46"),
        literals: &[
            UiLiteral {
                offset: 4607,
                source: "Motion Tracker",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 4626,
                source: "Beeps when Motion is Detected",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Painkillers.uexp",
        md5: "9EA1906CE954D0D288D8BED27D422C18",
        header_md5: Some("DB136E5AFFECF9CE7C98F975711E650B"),
        literals: &[
            UiLiteral {
                offset: 4689,
                source: "Painkillers",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 4705,
                source: "(LMB) to Take Painkillers",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Pickaxe.uexp",
        md5: "EE1808AA96BA339301C811173BC79C0F",
        header_md5: Some("0B5BCA089C657E6A170F196253B27753"),
        literals: &[
            UiLiteral {
                offset: 20089,
                source: "Pickaxe",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 20125,
                source: "(LMB) to swing",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_PoolCue.uexp",
        md5: "6F1105D610FFCFBF51FE839DC9CAAF37",
        header_md5: Some("D5F438CBE730BB702129637DC21EE0FE"),
        literals: &[
            UiLiteral {
                offset: 11201,
                source: "Pool Cue",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 11238,
                source: "(LMB) To shoot.",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Radio.uexp",
        md5: "3D08974A0153B91401826B2082292B8C",
        header_md5: Some("E51BE9D1D49D027D9F4ABB56D5F8A86A"),
        literals: &[
            UiLiteral {
                offset: 1240,
                source: "Radio",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 1250,
                source: "Hold (LMB) to Talk into Radio",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Revolver.uexp",
        md5: "E4D4EBEA834AF70BCA0F7DA092A319F4",
        header_md5: Some("59D80897B26A013214E0E064D5EEDECC"),
        literals: &[
            UiLiteral {
                offset: 5333,
                source: "Revolver",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 5370,
                source: "(LMB) to shoot",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Shotgun.uexp",
        md5: "5D1D3D1AF45F189A67FA0F2D046C9C18",
        header_md5: Some("24F3E535265500878D6E8EB3FCD116DE"),
        literals: &[
            UiLiteral {
                offset: 5335,
                source: "Shotgun",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 5371,
                source: "(LMB) to shoot",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Sniper.uexp",
        md5: "C37BB14BEB864A19E2C1A0F0B55024C1",
        header_md5: Some("1B4E56FE66B17584927671DA74453315"),
        literals: &[
            UiLiteral {
                offset: 5335,
                source: "Sniper",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 5370,
                source: "(LMB) to shoot (RMB) to aim",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_Syringe.uexp",
        md5: "8611339C5FCC6716EBEC667A5CC70031",
        header_md5: Some("F43FD7651B7781831FD18116394B323A"),
        literals: &[
            UiLiteral {
                offset: 4691,
                source: "Vaccine",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 4703,
                source: "Press (LMB) to use Vaccine",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/Item_mp5.uexp",
        md5: "24860A6ADE52D0DAC1040B4BC0AD2968",
        header_md5: Some("F31BFADD548187D6552E3976BF8E925D"),
        literals: &[
            UiLiteral {
                offset: 5335,
                source: "MP5K",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 5368,
                source: "(LMB) to shoot",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
    EmbeddedUiAsset {
        path: "NuclearNightmare/Content/Gameplay/Items/PoolCue/PoolCue.uexp",
        md5: "354EC2260780D3DB2F5D7F40A2ACB9E2",
        header_md5: Some("D6CD604C7A693EA59AFAE8632A5BE138"),
        literals: &[
            UiLiteral {
                offset: 1320,
                source: "Pool Cue",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: 1333,
                source: "(LMB) To Shoot",
                layout: UiLiteralLayout::FString,
            },
        ],
    },
];

fn read_embedded_ui_asset(asset: &EmbeddedUiAsset, path: &Path) -> Result<Vec<u8>> {
    let data = fs::read(path)?;
    if format!("{:X}", md5::compute(&data)) != asset.md5 {
        bail!(
            "内嵌 UI 资源已变化，须重新核实文本和蓝图边界：{}",
            asset.path
        );
    }
    for literal in asset.literals {
        let parsed = match literal.layout {
            UiLiteralLayout::StringAssignment { .. } => read_script_string(&data, literal.offset),
            UiLiteralLayout::FString => read_fstring_at(&data, literal.offset),
        };
        if parsed.as_ref().map(|(text, _)| text.as_str()) != Some(literal.source) {
            bail!("内嵌 UI 原文不匹配：{} / {}", asset.path, literal.source);
        }
    }
    Ok(data)
}

fn embedded_ui_rows(extracted: &BTreeMap<String, PathBuf>) -> Result<Vec<CsvRow>> {
    let mut rows = Vec::new();
    for asset in EMBEDDED_UI_ASSETS {
        read_embedded_ui_asset(
            asset,
            extracted.get(asset.path).context("缺少内嵌 UI 资源")?,
        )?;
        if asset.header_md5.is_some() {
            read_item_ui_header(asset, extracted)?;
        }
        for literal in asset.literals {
            rows.push(CsvRow {
                namespace: ASSET_TEXT_NAMESPACE.into(),
                key: asset_text_key(asset.path, literal.source, 0),
                hash: unreal_text_crc(literal.source),
                text: literal.source.into(),
                trans_text: String::new(),
                context_asset: asset.path.into(),
            });
        }
    }
    Ok(rows)
}

fn patch_embedded_ui(
    extracted: &BTreeMap<String, PathBuf>,
    build: &Path,
    rows: &[CsvRow],
) -> Result<()> {
    for asset in EMBEDDED_UI_ASSETS {
        let mut data = read_embedded_ui_asset(
            asset,
            extracted.get(asset.path).context("缺少内嵌 UI 资源")?,
        )?;
        let asset_rows: Vec<_> = rows
            .iter()
            .filter(|row| row.namespace == ASSET_TEXT_NAMESPACE && row.context_asset == asset.path)
            .collect();
        if asset_rows.len() != asset.literals.len() {
            bail!("内嵌 UI CSV 记录数不匹配，请重新 decode：{}", asset.path);
        }
        let translations: Vec<&str> = asset
            .literals
            .iter()
            .map(|literal| {
                let key = asset_text_key(asset.path, literal.source, 0);
                let row = asset_rows
                    .iter()
                    .find(|row| row.key == key)
                    .context("缺少内嵌 UI 译文")?;
                if row.text != literal.source
                    || row.hash != unreal_text_crc(literal.source)
                    || row.trans_text.contains('\0')
                {
                    bail!("内嵌 UI 原文、哈希或译文无效：{}", literal.source);
                }
                Ok(row.trans_text.as_str())
            })
            .collect::<Result<_>>()?;
        if asset.header_md5.is_some() {
            let mut header = read_item_ui_header(asset, extracted)?;
            replace_item_properties(&mut header, &mut data, asset.literals, &translations)?;
            let target = build.join(asset.path.replace(".uexp", ".uasset"));
            fs::create_dir_all(target.parent().context("物品头文件目录无效")?)?;
            fs::write(target, header)?;
        } else {
            replace_ui_assignments(&mut data, asset.literals, &translations)?;
        }
        let target = build.join(asset.path);
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
    if Some(format!("{:X}", md5::compute(&data)).as_str()) != asset.header_md5 {
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
                source: "NIGHT ",
                layout: UiLiteralLayout::StringAssignment { start: 0, end },
            },
            UiLiteral {
                offset: 47,
                source: "DAY ",
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
                source: "MP5K",
                layout: UiLiteralLayout::FString,
            },
            UiLiteral {
                offset: hint_at,
                source: "(LMB) to shoot",
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
    fn embedded_ui_rows_are_allowed_but_unknown_assets_are_rejected() -> Result<()> {
        let mut rows = Vec::new();
        for asset in EMBEDDED_UI_ASSETS {
            for literal in asset.literals {
                rows.push(CsvRow {
                    namespace: ASSET_TEXT_NAMESPACE.into(),
                    key: asset_text_key(asset.path, literal.source, 0),
                    hash: unreal_text_crc(literal.source),
                    text: literal.source.into(),
                    trans_text: "测试".into(),
                    context_asset: asset.path.into(),
                });
            }
        }
        validate_rows(&rows, true)?;
        rows[0].context_asset = "NuclearNightmare/Content/Unknown.uexp".into();
        assert!(validate_rows(&rows, true).is_err());
        Ok(())
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
            source: " has powered on the ",
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
            source: "(Q) To Drop ",
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
 let game=resolve_game_dir(Path::new(".."))?;prepare_oodle(&game)?;
 let path="NuclearNightmare/Content/Graphics/Computer/NukeComputer_UI";
 let files=extract_entries(&game.join(ORIGINAL_PAK),&[format!("{path}.uasset"),format!("{path}.uexp")],Path::new("target/computer-ui"))?;
 let h=fs::read(&files[&format!("{path}.uasset")])?; let d=fs::read(&files[&format!("{path}.uexp")])?;
 println!("uasset MD5 {:X}; uexp MD5 {:X}",md5::compute(&h),md5::compute(&d));
 for e in cooked_item_exports(&h,&d)?.0 { if e.is_default_object {println!("CDO {}..{}",e.start,e.end);}}
 for at in [37882,37969,38154] {println!("{at}: {:?}; prefix {:02X?}",read_fstring_at(&d,at), &d[at-60..at]);}
 let folder=read_fstring_at(&h,52).unwrap().1; let s=folder+4;
 let mut names=Vec::new();let mut pos=u32_at(&h,s+4)? as usize;
 for _ in 0..u32_at(&h,s)? {let (name,end)=read_fstring_at(&h,pos).unwrap();names.push(name);pos=end+4;}
 let stride=(u32_at(&h,s+28)?-u32_at(&h,s+36)?)/u32_at(&h,s+32)?;
 let imp=u32_at(&h,s+36)?+94*stride;println!("Import -95 {}",names[u32_at(&h,imp as usize+20)? as usize]);
 Ok(())
}