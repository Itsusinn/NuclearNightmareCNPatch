# Nuclear Nightmare 简体中文补丁

为 Nuclear Nightmare 提供简体中文翻译和中文字体，覆盖英文语言槽。部分游戏文本仍保留英文。

当前补丁版本：**v0.1.6**，适配 **抢先体验版 V8.1.4 / Steam build 26341650**。

本版调整补丁文件名，同时包含游戏版本和补丁版本；译文及补丁内容与 v0.1.5 一致。

本次重新提取 1761 条文本，更新主菜单版本号与结算界面变更的文本键。全部译文已通过原文、占位符、字体覆盖与严格打包回读校验；生成器与审校工具共 31 项测试通过。已进游戏检查 V8.1.4 启动画面、主菜单、画面与显示设置、建房及世界设置；教程、任务日志、地图、物品、专长、加油及结算界面尚未完成本版本实测。部分游戏文本仍保留英文。

## 补丁安装

仓库不跟踪 PAK 文件。补丁可从 [Releases](https://github.com/Itsusinn/NuclearNightmareCNPatch/releases) 下载，或按 [AGENTS.md](./AGENTS.md) 在本地生成。

1. 退出游戏，将 `zz_NuclearNightmare_Chinese_P_V8.1.4-v0.1.6.pak` 放入游戏安装目录下的 `NuclearNightmare\Content\Paks\` 文件夹；更新补丁时先移除旧版本汉化 PAK（包括旧名 `zz_NuclearNightmare_Chinese_P.pak`），只保留一个版本。
2. 启动游戏，在“选项 → 显示 → 语言”中选择“简体中文”（补丁覆盖原 `English` 语言槽，未安装时显示为 `English`）。
3. 重启游戏，使汉化生效。

卸载时，退出游戏并删除该补丁 PAK 即可。

## 字体版权与许可

补丁使用 **Sarasa Gothic SC（更纱黑体）常用中文子集**，基于 1.0.42 版精简，字体大小约 3.90 MiB。原项目：[be5invis/Sarasa-Gothic](https://github.com/be5invis/Sarasa-Gothic)。子集覆盖当前汉化文本、GB2312 常用中文及部分其他字符，不保证罕见字或任意玩家名字、聊天内容的完整显示。

Copyright © 2015–2025 Renzhi Li（Belleve Invis）。部分字体内容版权属于 The Inter Project Authors（2016）、Adobe Systems Incorporated（2014–2021）及 Google Inc.（2012）。完整版权及子集说明见 [fonts/NOTICE.txt](./fonts/NOTICE.txt)。

该字体依据 [SIL Open Font License 1.1（完整许可文本）](./fonts/OFL.txt) 授权。再分发时须保留版权声明和许可信息；字体本身继续适用 OFL 1.1，不适用本项目代码的 AGPL-3.0 许可证。补丁 PAK 中同时附带字体许可及版权说明。
