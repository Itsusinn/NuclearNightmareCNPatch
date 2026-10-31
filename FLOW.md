# 翻译操作流程

按顺序执行，当前步骤成功后再继续。

## 1. 同步

进入 `NuclearNightmareCNPatch/`，通过 Steam 更新游戏并退出游戏。

```powershell
git status --short
git pull --ff-only
```

## 2. 提取

```powershell
just decode
just review-prepare
```

## 3. 翻译

1. 填写 `review.csv` 的 `trans_text`，保留其余字段、占位符和必要换行，保存为 UTF-8 CSV。
2. 修改既有译文时，从最新 `origin.csv` 复制对应完整行到 `review.csv`，再填写译文。

## 4. 校验并合并

```powershell
just review-check
```

若提示 `Missing font glyph`，先执行下方“4.1 缺字处理”；校验通过后继续：

```powershell
just review-apply
git diff -- translations.csv review.csv
```

确认合并结果后，归档本批 `review.csv` 并将其清理为仅表头。

### 4.1 缺字处理

在仓库根目录依次执行以下步骤，保留已填写的 `review.csv`，完成前不要清空审校表。

1. 使用完整原字体校验本批译文，并输出完整暂存翻译库：

```powershell
cargo +nightly -Zscript scripts/review_translations.rs apply --decoded origin.csv --translations translations.csv --review review.csv --font fonts/SarasaGothicSC-Regular.ttf --output target/font-rebuild/translations-reviewed.csv
```

若仍提示 `Missing font glyph`，修改报错记录的译文，换用原字体支持的字符，再重新执行此命令。

2. 使用暂存翻译库生成字体子集，填写以下三个路径参数：

```powershell
cargo +nightly -Zscript scripts/subset_font.rs --source fonts/SarasaGothicSC-Regular.ttf --translations target/font-rebuild/translations-reviewed.csv --output-dir target/font-rebuild/generated
```

3. 使用生成的常用中文子集重新校验本批译文，查看报告和字形对照图：

```powershell
cargo +nightly -Zscript scripts/review_translations.rs check --decoded origin.csv --translations translations.csv --review review.csv --font target/font-rebuild/generated/NNChinese-GB2312-Regular.ttf
Get-Content -LiteralPath target/font-rebuild/generated/report.json -Encoding UTF8
Invoke-Item -LiteralPath target/font-rebuild/generated/comparison.png
```

确认报告中两种子集的 `missing_corpus_characters` 均为空，以下四项均为 `true`，并检查对照图中的字形：

- `sample_raster_matches_original`
- `horizontal_metrics_match_original`
- `global_hinting_tables_match_original`
- `all_retained_outlines_and_hint_instructions_match_original`

4. 备份正式字体和来源声明，再替换为通过校验的常用中文子集：

```powershell
$fontBackupDir = Join-Path 'target/font-rebuild' ('backup-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $fontBackupDir -Force | Out-Null
Copy-Item -LiteralPath fonts/NNChinese-GB2312-Regular.ttf -Destination $fontBackupDir
Copy-Item -LiteralPath fonts/NOTICE.txt -Destination $fontBackupDir
Copy-Item -LiteralPath target/font-rebuild/generated/NNChinese-GB2312-Regular.ttf -Destination fonts/NNChinese-GB2312-Regular.ttf -Force
```

5. 读取新字体的大小、哈希、字符及生成器信息：

```powershell
$fontReport = Get-Content -LiteralPath target/font-rebuild/generated/report.json -Raw -Encoding UTF8 | ConvertFrom-Json
$fontReport | Select-Object source_sha256, translations_sha256, corpus_rows, corpus_characters, subsetter
$fontReport.variants | Where-Object { $_.file -eq 'NNChinese-GB2312-Regular.ttf' } | Format-List file, bytes, sha256, unicode, glyphs
Get-FileHash -LiteralPath fonts/NNChinese-GB2312-Regular.ttf -Algorithm SHA256
```

更新 `fonts/NOTICE.txt` 中的 `Subset file`、`Subset SHA-256`、`Generated using`、翻译库行数和来源标识、Unicode 映射数及字形数；将旧 fontTools 生成器记录改为报告中的 `skera 0.9.0`，将旧翻译库提交号改为本次暂存翻译库的 SHA-256。保留字体版权、OFL 许可、字符范围和修改后的字体族名称。

6. 回到步骤 4，执行 `just review-check`、`just review-apply` 并检查差异；随后执行步骤 5、6，重新打包并检查新增字符在实际界面中的显示。

## 5. 打包并安装

退出游戏，将旧补丁备份到 PAK 目录之外，然后执行：

```powershell
just encode translations.csv
Get-FileHash -LiteralPath ../NuclearNightmare/Content/Paks/zz_NuclearNightmare_Chinese_V8.1.4-v0.1.6_P.pak -Algorithm SHA256
```

## 6. 游戏验证

1. 启动游戏，选择“选项 → 显示 → 语言 → 简体中文”，重启游戏。
2. 检查本次修改界面，抽查主菜单、设置、教程、任务日志、地图标签、物品和专长。
3. 检查占位符、缺字、乱码、截断及排版。
4. 记录游戏版本、Steam build、PAK SHA-256、已验证界面、未验证场景和截图。
5. 游戏再次更新后，从提取步骤重新执行。

## 7. 提交与发布

```powershell
cargo +nightly test -Zscript --manifest-path scripts/nuclear_cn.rs --jobs 2
cargo +nightly test -Zscript --manifest-path scripts/review_translations.rs --jobs 2
cargo +nightly test -Zscript --manifest-path scripts/subset_font.rs --jobs 2
git fsck --full
git diff --check
git status --short
```

1. 按本次修改范围暂存正式译文、脚本、文档、字体及许可声明，提交并推送。
2. 将已验证的 PAK 上传为 Release 附件，填写适配版本和未翻译范围。
3. 核对附件大小及 SHA-256。
