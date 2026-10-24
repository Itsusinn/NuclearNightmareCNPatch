# 脚本目录

所有手写脚本源码放在此目录并纳入 Git，数据、报告和构建产物写入 `target/`。以下命令从仓库根目录执行；路径参数也可以使用绝对路径。

- `nuclear_cn.rs`：提取翻译、自动发现候选文本、严格生成补丁。
- `subset_font.rs`：生成并验证中文字体子集，替代历史 Python 实现。
- `verify_discovery.rs`：只读比对两次构建的文件集合及逐字节内容，校验重新提取的 CSV 完整保留基准记录，并报告新增候选数。可额外要求当前翻译库与基准一致；重复键、缺列、空基准、丢失或修改记录均会失败。
- `fill_discovery_candidates.rs`：2026-10-24 首批 15 条候选的审定译文录入脚本；核对提取结果、保留已有记录并检查字体覆盖。运行 `cargo +nightly -Zscript scripts/fill_discovery_candidates.rs <提取.csv> <翻译库.csv>`。单位 `mph`、`ft/s` 保留原写法，地图起终点和仪表前缀采用适合现有槽位的简短中文；以后新增候选需另行审定。
- `archive/`：以往调查、修补脚本及源码快照，见[历史代码说明](./archive/README.md)。

自动发现改造的文件级回归验证示例（需要先保留基准数据并生成新版输出）：

```powershell
cargo +nightly -Zscript scripts/verify_discovery.rs --before-build target/computer-ui/encode/build --after-build target/auto-discovery/encode/build --baseline-csv target/auto-discovery/baseline.csv --decoded-csv target/auto-discovery/decoded.csv --current-csv translations.csv
```

工具不写死文件数量或本机路径，仅读取传入的数据。此比对不能代替游戏内验证。
