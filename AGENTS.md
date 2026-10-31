# Nuclear Nightmare 汉化 CLI

脚本文件：[scripts/nuclear_cn.rs](./scripts/nuclear_cn.rs)

所有手写脚本（包括临时调查、验证、数据修补脚本）必须放在 `scripts/` 下，以便纳入 Git 版本管理；`target/` 只存放提取数据、报告、缓存和构建产物，不保存唯一的脚本源码。历史调查代码放在 `scripts/archive/`，使用限制见其 README。新增资源引用使用项目相对路径或命令行参数，禁止写死本机安装路径。

这是一个纯 Rust 单文件 Cargo CLI，提供 `decode` 和 `encode` 两个子命令。当前电脑已经安装 Rust nightly，因此直接使用 Cargo 官方单文件脚本功能即可，无需安装第三方 `cargo-script`、`repak.exe`、.NET 或 LocResUtility。

PAK 读写使用固定提交的 [`trumank/repak`](https://github.com/trumank/repak)，LocRes 读写使用固定版本的 [`locres-rs`](https://crates.io/crates/locres-rs)。

## 仓库大文件与字体复现

字体（`*.ttf`）作为二进制文件直接纳入普通 Git，克隆后即可编译，无需安装 Git LFS 或单独下载字体。原始字体 `fonts/SarasaGothicSC-Regular.ttf` 和同目录下的子集均适用 `fonts/OFL.txt` 中的许可。

PAK 文件与 `dist/` 分发目录仅保留在本地，不纳入 Git 或 Git LFS。发布补丁时须重新严格打包并进游戏验证。`target/` 中的中间资源、旧包和缓存，以及 `origin.csv`、Oodle 运行库不提交。

字体精简脚本 `scripts/subset_font.rs` 使用纯 Rust 的 Skera 0.9.0、fontdue 0.9.5，无需 Python、uv、fontTools 或 Pillow。通过 Cargo nightly 单文件脚本运行：

```powershell
cargo +nightly -Zscript scripts/subset_font.rs
# 或
just subset-font
```

默认读取 `fonts/SarasaGothicSC-Regular.ttf` 和 `translations.csv`，在 `target/font-subsets/` 生成常用中文/UI 两种子集、字符覆盖与哈希报告 `report.json`、字形对照图 `comparison.png`；默认路径相对于脚本所在项目，与运行目录无关。可用 `--source`、`--translations`、`--output-dir` 指定路径。脚本仅支持静态 TrueType 字体，检查字符覆盖、全部保留字形的轮廓和 hinting 指令、水平字宽、全局 hinting 表、字体校验和，以及样本文字在 16/24/40 像素下的无 hinting 栅格化一致性。两种子集均通过后才写入文件，不直接覆盖正式字体。

Skera 与原 fontTools 的二进制布局不同，输出哈希不保证相同；保留推荐字形时会额外保留 U+0000 映射。现有正式子集的 fontTools 来源记录仍保留在 `fonts/NOTICE.txt` 中。采用新输出时，将常用中文版复制到 `fonts/NNChinese-GB2312-Regular.ttf`，同步该声明中的生成器、大小、哈希和字符信息，并重新构建及进游戏验证 PAK。

脚本回归测试：`cargo +nightly test -Zscript --manifest-path scripts/subset_font.rs --jobs 2`。

## 使用 just（推荐）

工程位于游戏根目录下的 `NuclearNightmareCNPatch` 文件夹。进入该目录后可以直接运行：

```powershell
cd .\NuclearNightmareCNPatch
just decode
just encode
```

默认使用 `origin.csv`，并将补丁输出到游戏的 PAK 目录。也可以覆盖路径：

```powershell
just decode .\待翻译.csv
just encode .\已翻译.csv .\输出\Chinese_P.pak
```

`just encode` 默认启用严格校验。若需要让空白译文自动回退为英文，可运行：

```powershell
just encode-loose
```

## 1. 提取待翻译 CSV

在 `NuclearNightmareCNPatch` 工程目录运行：

```powershell
cargo +nightly -Zscript .\scripts\nuclear_cn.rs decode --output .\origin.csv
```

`decode` 会：

1. 从原始 `NuclearNightmare-Windows.pak` 单独提取英文 `Game.locres`。
2. 自动发现并提取游戏的主菜单、建房、服务器列表、服装、背包、地图、任务、结算、技能树、按键设置、聊天、语音设置、载具 HUD 和教程弹窗等 UI `.uexp`，修正游戏更新后失效的文本哈希。专长名称与效果还会扫描 `HexTree/Blueprints/Enums/` 和 `HexTree/DataTables/`，避免仅扫描控件而遗漏实际显示的数据。
3. 补入这些当前界面中存在、但旧 `locres` 没有的文本。支持带命名空间及可读键的 Base FText（如 `NNUIRemaster`），以及 Blueprint `EX_TextConst/LocalizedText` 中的运行时文本（如动态延迟提示）；以完整 `namespace/key` 区分记录。
4. 自动枚举 UI、物品、发电机资源及配套头文件，从当前包的名称表、导入表、导出表计算范围和偏移；不再维护逐条资源路径/原文/偏移/版本 MD5 清单。在 Function 导出中解析 `Concat_StrStr` / `SelectString` 的简单字符串赋值，在类默认对象（CDO）内识别符合 FString 序列化格式的人类可读文本，并排除已有本地化 FText 的命名空间、键及原文。CDO 探测属于有范围限制的候选发现，不是完整的 Unreal unversioned 属性 schema 解析；新候选仍需人工确认用途、填写译文。
   保留少量规则：可回写 CDO 属性限于物品和电脑界面，排除测试物品及弹药默认值；主菜单/设置的脚本字符串仅报告，避免修改参与选项匹配的值。教程字幕按实际导入函数 `KismetSystemLibrary.SetStringPropertyByName` 与 `DescText` 属性识别；教程地图标签限定 `LevelLocationMap_C` 实例，排除对象名，定位首个可读标签 FString。其余表达式不猜测跳转结构。
   支持回写的候选以 `__ASSET_TEXT__` 记录加入 CSV；键按资源路径、原文、同文出现次序计算，移动偏移不会改变键，原有译文可复用。旁边同时生成 `<输出文件名>.candidates.csv`，列出资源、导出对象、类型、起止偏移、原文、状态和理由：`candidate` 表示支持结构、待确认翻译，`review_only` 仅供审查，`unsupported` 表示资源布局尚不支持。报告不会直接作为补丁输入；不在翻译 CSV 中的候选保留原文。
5. 先从随项目提供的 `translations.csv` 匹配译文，再保留输出路径已有 CSV 中的非空译文。两者都要求 `namespace/key`、源文本哈希和原文匹配；原文变化的记录需要重新翻译。
6. 生成带 UTF-8 BOM 的 `origin.csv`，可以直接用 Excel 或文本编辑器打开。

不会解包完整的游戏 PAK。扫描同一个 `namespace/key` 时若发现不同原文，会报错，避免生成互相冲突的本地化条目。

任务日志另外扫描 `Gameplay/ClassifiedMaterials/` 和 `Maps/` 下实际带 `.umap` 配套头的地图：地图内仅从 `ObjectiveManager_C` 实例提取本地化 FText（任务地点、描述），不修改任务标识；非教程地图的 `LevelLocationMap_C` 实例仅提取首个英文显示 FString，保留后续俄文标签及对象名。地图标签通过通用属性回写同步更新 `.umap` 导出长度、偏移和 `BulkDataStartOffset`，逐个校验未修改导出的字节。任务 FText 仍只写入 `Game.locres`，不会覆盖任务对象。地图文件由原包枚举，无逐地图路径或偏移清单。

CSV 字段：

| 字段 | 用途 |
|---|---|
| `namespace` | Unreal 本地化命名空间；`__ASSET_TEXT__` 表示需要直接回写资源的内嵌文本 |
| `key` | FText 唯一键 |
| `hash` | 当前源文本的 Unreal CRC32 |
| `text` | 英文原文；不要修改 |
| `trans_text` | 译文填写列 |
| `context_asset` | 能定位到时，记录文本所在 UI 资产 |

翻译人员只需填写 `trans_text`。请保留 `{type}`、`{1}` 等占位符和必要换行。

`origin.csv` 是本地提取快照，`translations.csv` 是纳入版本管理并随生成器编译的正式翻译库。待审译文使用同样六列的 `review.csv`，不要把逐条译文或键清单写进 Rust 程序。

### 通用审校流程

先运行 `decode` 获得当前游戏的提取结果，然后执行：

```powershell
just review-prepare
# 编辑 review.csv，只填写 trans_text
just review-check
just review-apply
# 从正式翻译库生成补丁
just encode translations.csv
```

这些命令调用 `cargo +nightly -Zscript scripts/review_translations.rs prepare|check|apply`。默认读取 `origin.csv`、`translations.csv`、`review.csv`，校验字体使用 `fonts/NNChinese-GB2312-Regular.ttf`；可用 `--decoded`、`--translations`、`--review` 和 `--font`（check/apply）覆盖路径。`apply` 必须显式指定 `--output`，可以输出到正式翻译库或暂存 CSV。所有相对路径相对于运行目录。

- `prepare` 为新增、原文/哈希变化或缺译的记录生成空译文行。若已有非空审校表，会停止，避免覆盖尚未处理的译文。当前没有待审条目时，`review.csv` 只保留表头。需要修改既有译文时，也可从最新提取结果复制相应行到审校表。
- `check` 只读校验完整合并结果。以 `namespace/key` 匹配，要求审校表中的原文和哈希与提取结果完全一致；内嵌资源文本还须匹配资源路径。检查重复键、未知键、缺失的旧键、未审定的新增/变更、空译文、占位符名称及次数、中文字体缺字。提取结果中的译文不会自动覆盖正式库。
- `apply` 执行相同校验，全部通过后写入同目录临时文件、回读确认，再原子替换输出；未涉及的正式记录保持原样。不会改写提取结果或审校表，失败时不覆盖输出。反复应用相同审校内容得到相同记录。

合并后，确认结果并将 `review.csv` 清理为表头，或将本批数据另存归档；正式译文只维护在 `translations.csv`。工具校验的是给定提取快照，不能判断游戏是否已更新，更新后须重新 `decode`。审校也不代替 `encode --strict` 对内嵌文本空间和资源结构的检查。

通用工具测试：`cargo +nightly test -Zscript --manifest-path scripts/review_translations.rs --jobs 2`。原 `fill_discovery_candidates.rs` 与 `fill_perk_translations.rs` 的译文已经合入正式库，其一次性脚本已由此流程取代。

## 2. 编译汉化 PAK

```powershell
cargo +nightly -Zscript .\scripts\nuclear_cn.rs encode --input .\origin.csv --strict
```

默认输出：

```text
NuclearNightmare\Content\Paks\zz_NuclearNightmare_Chinese_P_V8.1.4-v0.1.6.pak
```

默认文件名格式为 `zz_NuclearNightmare_Chinese_P_<游戏版本>-v<补丁版本>.pak`；游戏版本记录在 `scripts/nuclear_cn.rs` 的 `DEFAULT_PATCH_PAK` 中，补丁版本来自同文件的 `[package].version`。适配新游戏版本或发布新补丁时同步更新相应值及文档示例。`just encode` 与 `just encode-loose` 共用生成器的默认文件名，显式指定输出路径时使用指定名称。更新安装时先将旧版本汉化 PAK 移出游戏 PAK 目录，只保留一个版本。

`encode` 会：

1. 校验 CSV 键、重复项和 `{...}` 占位符的名称及出现次数。
2. 将普通 CSV 记录编译为 Unreal `Game.locres`。
3. 只覆盖英文语言槽 `en`；俄文及其他语言槽保持游戏原版。
4. 使用随项目提供的 `fonts/NNChinese-GB2312-Regular.ttf`（Sarasa Gothic SC 1.0.42 常用中文子集）生成 11 个中文字体覆盖。原游戏的 `NotoSansSC-Regular.ufont` 仅用作容器格式校验模板；其字体内容会替换为 Sarasa 子集。
5. 按翻译 CSV 涉及的资源重新扫描，独立验证每条 `__ASSET_TEXT__` 的键、原文、哈希和结构；不信任上次扫描的偏移，无法重新定位的记录直接报错。扫描过程中计算的文件指纹仅防止同一次运行内源文件被改动，不锁定某个游戏版本。`encode --work-dir` 下保留 `candidates.csv` 供诊断。
   字符串拼接和选择赋值先以 Unicode 等长改写，在完整赋值之后补空操作，保留变量参数、条件及后续指令偏移。多个字面量共享同一赋值空间；译文超出可用空间时报错，不截断或猜测跳转修复。教程奇数字节字幕保留独立 void 属性赋值调用的特殊处理；地图标签优先按原属性槽等长写入 UTF-16，终止符前补空格；超长标签仅在已识别的 `LevelLocationMap_C` 实例属性内扩容，同步调整 `.umap` 的导出长度、后续偏移及 `BulkDataStartOffset`，逐个比对其他导出，并将 `.umap` 与 `.uexp` 成对打包。
   CDO FString 属性可以按 UTF-16 实际长度扩缩，同步更新配套 `.uasset` 的导出长度、后续导出偏移和 `BulkDataStartOffset`。同一资源同时包含脚本和属性时先改脚本，再扩缩属性；检查默认对象边界、连续导出、包尾标记、译文回读，逐个比对其他导出。未选中或回退英文的文本保留原始字节。
6. 自动读取并保留原 PAK 的版本、mount point 和 path-hash seed，默认使用 Zlib 压缩生成最终 PAK；传入 `--no-compression` 可恢复无压缩打包。
7. 将 `fonts/OFL.txt` 和 `fonts/NOTICE.txt` 一并打包到 `NuclearNightmare/Content/Localization/ThirdParty/SarasaGothic/`。条目数量由实际生成文件计算，在 Rust 进程内回读 PAK、校验元数据，并逐项解压，与打包前的文件比较完整内容。

`--strict` 会在输入 CSV 的任何 `trans_text` 留空时停止；它不要求翻译候选报告中的所有条目。省略该参数时，空白译文会自动回退到英文原文。

字体项目：[be5invis/Sarasa-Gothic](https://github.com/be5invis/Sarasa-Gothic)。随项目提供的子集约 3.90 MiB，保留当前翻译库字符、GB2312 常用中文和部分其他字符；完整来源、字符范围与哈希见 `fonts/NOTICE.txt`。新增译文时需检查字符覆盖，必要时重新生成子集。字体适用 OFL 1.1，不能改用代码的 AGPL 许可证；分发时保留 `fonts/OFL.txt` 和版权声明。

严格校验只覆盖提取到的文本，不代表游戏全部文本均已汉化。V8.1.3 已为画质、窗口模式和建房选项提供 `NNMenuOptions` 本地化键，启动提示也已加入本地化表，本补丁通过译文库覆盖这些文本。候选报告中仅供审查的设置脚本字符串仍可能参与选项匹配，不能仅替换显示数组。其他未提取资源仍需另行排查；每次游戏更新后应重新提取、打包并进游戏验证。

生成器回归测试：

```powershell
cargo +nightly test -Zscript --manifest-path scripts/nuclear_cn.rs --jobs 2
```

## 常用参数

```text
--game-dir <目录>       游戏根目录，默认工程目录的上一级（..）
--work-dir <目录>       保留中间文件，便于排错
```

原始游戏 PAK 使用 Oodle 压缩。Oodle 是专有压缩格式，因此 `repak` 会在进程内动态加载 `oo2core_9_win64.dll`：CLI 优先复用游戏目录中已有的运行库，缺失时由 `repak` 自动下载。该 DLL 是压缩算法运行库，不会启动外部工具进程；生成的汉化 PAK 默认使用 Zlib 压缩。

安装 PAK 后，请在游戏的“选项 → 显示 → 语言”中选择“简体中文”（原 `English` 槽），然后重启游戏。俄语将继续使用游戏原版俄文文本。
