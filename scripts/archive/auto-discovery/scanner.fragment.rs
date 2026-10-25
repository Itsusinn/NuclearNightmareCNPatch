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
        if count > header.len() / 8 { bail!("名称表长度无效"); }
        let mut names = Vec::with_capacity(count);
        for _ in 0..count {
            let (name, end) = read_fstring_at(header, cursor).context("名称表损坏")?;
            cursor = end.checked_add(4).context("名称表溢出")?;
            if cursor > header.len() { bail!("名称表越界"); }
            names.push(name);
        }
        let name_at = |at| -> Result<String> {
            let index = u32_at(header, at)? as usize;
            let number = u32_at(header, at + 4)?;
            let name = names.get(index).context("FName 索引无效")?;
            Ok(if number == 0 { name.clone() } else { format!("{name}_{}", number - 1) })
        };
        let import_count = u32_at(header, summary + 32)? as usize;
        let import_at = u32_at(header, summary + 36)? as usize;
        let export_at = u32_at(header, summary + 28)? as usize;
        if import_count > header.len() / 32 || import_at.checked_add(import_count * 32) != Some(export_at) {
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
        Ok(Self { names, imports, export_names, export_classes, exports })
    }

    fn object_name(&self, reference: i32) -> Option<&str> {
        if reference < 0 {
            self.imports.get(reference.unsigned_abs() as usize - 1).map(|v| v.0.as_str())
        } else if reference > 0 {
            self.export_names.get(reference as usize - 1).map(String::as_str)
        } else { None }
    }

    fn import_is(&self, reference: i32, owner: &str, function: &str) -> bool {
        reference < 0 && self.imports.get(reference.unsigned_abs() as usize - 1)
            .is_some_and(|(name, outer)| name == function && self.object_name(*outer) == Some(owner))
    }

    fn fname<'a>(&'a self, data: &[u8], at: usize) -> Option<&'a str> {
        if u32_at(data, at + 4).ok()? != 0 { return None; }
        self.names.get(u32_at(data, at).ok()? as usize).map(String::as_str)
    }

    fn field_path(&self, data: &[u8], at: usize) -> Option<usize> {
        let count = u32_at(data, at).ok()? as usize;
        if count == 0 || count > 16 { return None; }
        for i in 0..count { self.fname(data, at + 4 + i * 8)?; }
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
        if *data.get(start)? != 0x0f { return None; }
        let target = self.field_path(data, start + 1)?;
        if !matches!(*data.get(target)?, 0x00 | 0x01 | 0x02 | 0x48) { return None; }
        let call = self.simple_operand(data, target)?;
        if *data.get(call)? != 0x68 { return None; }
        let function = u32_at(data, call + 1).ok()? as i32;
        let arguments = if self.import_is(function, "KismetStringLibrary", "Concat_StrStr") { 2 }
            else if self.import_is(function, "KismetMathLibrary", "SelectString") { 3 }
            else { return None; };
        let mut at = call + 5;
        let mut literals = Vec::new();
        for _ in 0..arguments {
            if let Some((source, _)) = read_script_string(data, at) {
                if candidate_text(&source) {
                    literals.push(UiLiteral { offset: at, source, layout: UiLiteralLayout::StringAssignment { start, end: 0 } });
                }
            }
            at = self.simple_operand(data, at)?;
        }
        if *data.get(at)? != 0x16 { return None; }
        let end = at + 1;
        for literal in &mut literals { literal.layout = UiLiteralLayout::StringAssignment { start, end }; }
        Some((end, literals))
    }
}

fn candidate_text(text: &str) -> bool {
    let value = text.trim();
    (3..=4096).contains(&value.chars().count())
        && value.chars().any(|c| c.is_alphabetic())
        && !value.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        && !value.contains('\0')
        && !value.starts_with('/') && !value.contains("\\")
        && !value.contains("/Game/") && !value.contains("/Engine/")
        && !value.contains('_') && !value.contains("://")
        && !(value.len() >= 16 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        && !(value.contains('.') && !value.contains(' ') && !value.ends_with("..."))
}

fn property_literals(data: &[u8], start: usize, end: usize) -> Vec<UiLiteral> {
    let mut localized_sources = HashSet::new();
    for at in start..end.saturating_sub(4) {
        if let Some((_, _, _)) = read_base_text_at_key(&data[..end], at) {
            if let Some((_, source_at)) = read_fstring_at(&data[..end], at) { localized_sources.insert(source_at); }
        }
    }
    let mut output = Vec::new();
    let mut at = start;
    while at + 4 <= end {
        let length = i32::from_le_bytes(data[at..at + 4].try_into().unwrap()).unsigned_abs();
        if (4..=4097).contains(&length) && !localized_sources.contains(&at) {
            if let Some((source, tail)) = read_fstring_at(&data[..end], at) {
                if candidate_text(&source) {
                    output.push(UiLiteral { offset: at, source, layout: UiLiteralLayout::FString });
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
    path.ends_with(".uexp") && (is_ui_asset(path)
        || path.starts_with("NuclearNightmare/Content/Gameplay/Items/")
        || path.starts_with("NuclearNightmare/Content/Gameplay/Generators/"))
}

fn property_scope(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    (path.contains("/Gameplay/Items/") || path.contains("/Graphics/Computer/"))
        && !lower.contains("test") && !lower.contains("ammo")
}

fn discover_embedded_paths(pak_path: &Path) -> Result<Vec<String>> {
    let mut reader = BufReader::new(File::open(pak_path)?);
    let pak = repak::PakBuilder::new().reader(&mut reader)?;
    let files: HashSet<_> = pak.files().into_iter().collect();
    let mut paths: Vec<_> = files.iter().filter(|path| embedded_scope(path)
        && files.contains(&path.replace(".uexp", ".uasset"))).cloned().collect();
    paths.sort();
    Ok(paths)
}

fn scan_embedded_assets(extracted: &BTreeMap<String, PathBuf>, paths: &[String]) -> Result<(Vec<EmbeddedUiAsset>, Vec<TextCandidate>)> {
    let mut assets = Vec::new();
    let mut report = Vec::new();
    for path in paths {
        let data = fs::read(extracted.get(path).context("缺少扫描资源")?)?;
        let header_path = path.replace(".uexp", ".uasset");
        let header = fs::read(extracted.get(&header_path).context("缺少扫描资源头")?)?;
        let package = match PackageLayout::read(&header, &data) {
            Ok(package) => package,
            Err(error) => {
                report.push(TextCandidate { asset: path.clone(), export: String::new(), kind: "package".into(), offset: 0, end: data.len(), text: String::new(), status: "unsupported".into(), reason: error.to_string() });
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
                    } else { at += 1; }
                }
            }
            for literal in found {
                let patchable = kind == "string assignment" || allowed;
                let end = match literal.layout {
                    UiLiteralLayout::FString => read_fstring_at(&data, literal.offset).unwrap().1,
                    UiLiteralLayout::StringAssignment { end, .. } => end,
                };
                report.push(TextCandidate { asset: path.clone(), export: package.export_names[index].clone(), kind: kind.into(), offset: literal.offset, end, text: literal.source.clone(), status: if patchable { "candidate" } else { "review_only" }.into(), reason: if patchable { "结构可回写；填写 CSV 译文后才纳入补丁" } else { "范围外属性（包括设置逻辑、测试物品及弹药默认值），仅报告" }.into() });
                if patchable { literals.push(literal); }
            }
        }
        literals.sort_by_key(|literal| literal.offset);
        if !literals.is_empty() {
            let properties = literals.iter().any(|literal| matches!(literal.layout, UiLiteralLayout::FString));
            assets.push(EmbeddedUiAsset { path: path.clone(), md5: format!("{:X}", md5::compute(&data)), header_md5: properties.then(|| format!("{:X}", md5::compute(&header))), literals });
        }
    }
    Ok((assets, report))
}

fn write_candidates(path: &Path, candidates: &[TextCandidate]) -> Result<()> {
    let mut file = BufWriter::new(File::create(path)?);
    file.write_all(&[0xef, 0xbb, 0xbf])?;
    let mut writer = csv::Writer::from_writer(file);
    for candidate in candidates { writer.serialize(candidate)?; }
    writer.flush()?;
    Ok(())
}
