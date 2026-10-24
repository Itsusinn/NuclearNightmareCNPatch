# Nuclear Nightmare 简体中文补丁

为 Nuclear Nightmare 提供简体中文翻译和中文字体，覆盖英文语言槽。部分游戏文本仍保留英文。

当前补丁版本：**v0.1.2**，适配 **抢先体验版 V8.1.1 / Steam build 26172444**（2026-10-24 本机资源）。本次补齐任务日志、地图悬停地名、专长及物品提示等遗漏，并修复教程字幕与起终点标签；共 1,501 条翻译记录。地图覆盖均按当前游戏资源重新生成。

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

生成补丁时会再次扫描并校验所选文本，保留变量、条件和导出结构。设置逻辑字符串及不支持的蓝图表达式仍需另行处理；发布附件的内容以对应版本标签为准。

v0.1.2 已包含四张地图任务日志和悬停地名的提取：本批补入 124 条本地化文本及 32 个地图标签。任务文本从地图内的任务管理对象提取到翻译表；悬停地名仅修改地图标签的英文显示属性，并校验配套地图头和其他对象数据。

译文审校使用通用工具 `scripts/review_translations.rs`：提取后运行 `just review-prepare`，填写 `review.csv`，再运行 `just review-check` 和 `just review-apply`。正式译文维护在 `translations.csv`，不再为每批文本编写硬编码录入脚本。完整参数及校验范围见 [AGENTS.md](./AGENTS.md)。

## 字体版权与许可

补丁使用 **Sarasa Gothic SC（更纱黑体）常用中文子集**，基于 1.0.42 版精简，字体大小约 3.90 MiB。原项目：[be5invis/Sarasa-Gothic](https://github.com/be5invis/Sarasa-Gothic)。子集覆盖当前汉化文本、GB2312 常用中文及部分其他字符，不保证罕见字或任意玩家名字、聊天内容的完整显示。

Copyright © 2015–2025 Renzhi Li（Belleve Invis）。部分字体内容版权属于 The Inter Project Authors（2016）、Adobe Systems Incorporated（2014–2021）及 Google Inc.（2012）。完整版权及子集说明见 [fonts/NOTICE.txt](./fonts/NOTICE.txt)。

该字体依据 [SIL Open Font License 1.1（完整许可文本）](./fonts/OFL.txt) 授权。再分发时须保留版权声明和许可信息；字体本身继续适用 OFL 1.1，不适用本项目代码的 AGPL-3.0 许可证。补丁 PAK 中同时附带字体许可及版权说明。
