# Nuclear Nightmare 简体中文补丁

为 Nuclear Nightmare 提供简体中文翻译和中文字体，覆盖英文语言槽。部分游戏文本仍保留英文。

当前补丁版本：**v0.1.1**，适配 **抢先体验版 V8.1.1 / Steam build 26172444**（2026-10-24 本机资源）。本次同步版本号、新增蝙蝠的名称与说明及结算计数文本，共 1,266 条翻译记录；教程地图覆盖也按当前游戏资源重新生成。

## 补丁安装

仓库不跟踪 PAK 文件。补丁可从 [Releases](https://github.com/Itsusinn/NuclearNightmareCNPatch/releases) 下载，或按 [AGENTS.md](./AGENTS.md) 在本地生成。

1. 退出游戏，将 `zz_NuclearNightmare_Chinese_P.pak` 放入游戏安装目录下的 `NuclearNightmare\Content\Paks\` 文件夹；更新补丁时替换同名文件。
2. 启动游戏，在“选项 → 显示 → 语言”中选择 `English`。
3. 重启游戏，使汉化生效。

卸载时，退出游戏并删除该补丁 PAK 即可。

补丁生成和维护说明见 [AGENTS.md](./AGENTS.md)。

所有脚本源码统一保存在 [`scripts/`](./scripts/README.md)，包括生成器、字体工具和验证工具；运行产物继续写入被 Git 忽略的 `target/`。旧调查脚本和源码快照保留在 `scripts/archive/`。

## 开发中的自动文本发现

运行 `cargo +nightly -Zscript scripts/nuclear_cn.rs decode --output origin.csv` 会扫描当前游戏资源，生成翻译 CSV 和 `origin.candidates.csv` 候选报告。支持的界面字符串、物品/电脑默认属性及教程文本会自动计算结构和偏移；现有译文按键与原文匹配恢复。新增候选需确认用途并填写译文，报告中的 `review_only` / `unsupported` 条目不会自动改写。

生成补丁时会再次扫描并校验所选文本，保留变量、条件和导出结构。设置逻辑字符串及不支持的蓝图表达式仍需另行处理。此工具改动不代表已发布新的补丁版本。

## 字体版权与许可

补丁使用 **Sarasa Gothic SC（更纱黑体）常用中文子集**，基于 1.0.42 版精简，字体大小约 3.90 MiB。原项目：[be5invis/Sarasa-Gothic](https://github.com/be5invis/Sarasa-Gothic)。子集覆盖当前汉化文本、GB2312 常用中文及部分其他字符，不保证罕见字或任意玩家名字、聊天内容的完整显示。

Copyright © 2015–2025 Renzhi Li（Belleve Invis）。部分字体内容版权属于 The Inter Project Authors（2016）、Adobe Systems Incorporated（2014–2021）及 Google Inc.（2012）。完整版权及子集说明见 [fonts/NOTICE.txt](./fonts/NOTICE.txt)。

该字体依据 [SIL Open Font License 1.1（完整许可文本）](./fonts/OFL.txt) 授权。再分发时须保留版权声明和许可信息；字体本身继续适用 OFL 1.1，不适用本项目代码的 AGPL-3.0 许可证。补丁 PAK 中同时附带字体许可及版权说明。
