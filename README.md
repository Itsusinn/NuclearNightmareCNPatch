# Nuclear Nightmare 汉化 CLI

脚本文件：[nuclear_cn.rs](./nuclear_cn.rs)

这是一个纯 Rust 单文件 Cargo CLI，提供 `decode` 和 `encode` 两个子命令。当前电脑已经安装 Rust nightly，因此直接使用 Cargo 官方单文件脚本功能即可，无需安装第三方 `cargo-script`、`repak.exe`、.NET 或 LocResUtility。

PAK 读写使用固定提交的 [`trumank/repak`](https://github.com/trumank/repak)，LocRes 读写使用固定版本的 [`locres-rs`](https://crates.io/crates/locres-rs)。

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

在游戏根目录运行：

```powershell
cargo +nightly -Zscript .\nuclear_cn.rs decode --output .\origin.csv
```

`decode` 会：

1. 从原始 `NuclearNightmare-Windows.pak` 单独提取英文 `Game.locres`。
2. 自动发现并提取游戏的主菜单、建房、服务器列表、服装、背包、地图、任务、结算、技能树、按键设置、聊天、语音设置、载具 HUD 和教程弹窗等 UI `.uexp`，修正游戏更新后失效的文本哈希。
3. 补入这些当前界面中存在、但旧 `locres` 没有的文本；扫描范围不再依赖少量硬编码资产列表。
4. 扫描教程地图 Blueprint 中未进入 `Game.locres` 的内嵌文本，以特殊记录一并写入 CSV。
5. 输出路径已有 CSV 时，按 `namespace/key` 自动保留其中的非空译文。
6. 生成带 UTF-8 BOM 的 `origin.csv`，可以直接用 Excel 或文本编辑器打开。

不会解包完整的 9 GB 游戏 PAK。

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

## 2. 编译汉化 PAK

```powershell
cargo +nightly -Zscript .\nuclear_cn.rs encode --input .\origin.csv --strict
```

默认输出：

```text
NuclearNightmare\Content\Paks\zz_NuclearNightmare_Chinese_P.pak
```

`encode` 会：

1. 校验 CSV 键、重复项和 `{...}` 占位符。
2. 将普通 CSV 记录编译为 Unreal `Game.locres`。
3. 只覆盖英文语言槽 `en`；俄文及其他语言槽保持游戏原版。
4. 从原游戏提取 `NotoSansSC-Regular`，生成 11 个中文字体覆盖。
5. 根据 CSV 中的 `__ASSET_TEXT__` 记录，对教程地图内嵌文本进行等长汉化，保持 Blueprint 字节码偏移不变。
6. 自动读取并保留原 PAK 的版本、mount point 和 path-hash seed，然后生成最终 PAK。
7. 在 Rust 进程内回读最终 PAK，校验其元数据及 13 个文件（1 个英文本地化文件、11 个字体覆盖和 1 个教程地图补丁）。

`--strict` 会在任何 `trans_text` 留空时停止。省略该参数时，空白译文会自动回退到英文原文。

## 常用参数

```text
--game-dir <目录>       游戏根目录，默认工程目录的上一级（..）
--work-dir <目录>       保留中间文件，便于排错
```

原始游戏 PAK 使用 Oodle 压缩。Oodle 是专有压缩格式，因此 `repak` 会在进程内动态加载 `oo2core_9_win64.dll`：CLI 优先复用游戏目录中已有的运行库，缺失时由 `repak` 自动下载。该 DLL 是压缩算法运行库，不会启动外部工具进程；生成的汉化 PAK 使用无压缩条目。

安装 PAK 后，请在游戏的“选项 → 显示 → 语言”中选择 `English`，然后重启游戏。`Russian` 将继续使用游戏原版俄文文本。
