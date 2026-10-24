# 历史调查代码

此目录保留原先位于 `target/` 的 27 个调查脚本、旧生成器快照及源码片段，按原调查目录归档，供 Git 追踪和回溯。自动发现的通用比对脚本已整理为上一级的 `verify_discovery.rs`。

这些文件反映当时的资源结构、偏移和预期数量，未逐一适配当前游戏版本；部分包含旧生成器的完整副本。当前生成补丁请使用 `scripts/nuclear_cn.rs`，字体精简请使用 `scripts/subset_font.rs`。

- Rust 调查脚本从仓库根目录运行，读取的提取数据、CSV 和报告仍位于原来的 `target/` 路径；依赖这些本地数据的脚本不能在全新克隆中直接复现。
- `auto-discovery/baseline.rs` 是自动发现改造前的生成器快照；`scanner.fragment.rs` 是已合入正式生成器的早期源码片段，不能独立运行。
- `add.rs`、`add-translations.rs`、`fill.rs`、`expand-items.rs`、`format_csv.rs` 等历史脚本会修改翻译数据。它们保留用于追溯处理方式，复用前须审查其输入和写入目标；当前维护应使用正式生成器和已版本化的翻译库。
- `font-subsets/subset_font.py` 是已被 Rust 工具替代的旧实现，依赖 fontTools 和 Pillow；保留用于追溯正式字体的历史来源，输出仍写入 `target/font-subsets/`。

迁移只修正了源码位置相关的资源引用及旧 Python 工具的本机路径，未重跑这些历史数据修补操作。
