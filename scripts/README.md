# 脚本目录

所有手写脚本源码放在此目录并纳入 Git，数据、报告和构建产物写入 `target/`。以下命令从仓库根目录执行；路径参数也可以使用绝对路径。

- `nuclear_cn.rs`：提取翻译、自动发现候选文本、严格生成补丁。
- `inspect_perk_assets.rs`：诊断专长文本来源，运行 `<脚本> <原始 PAK> <输出目录>`，列出并提取专长相关资源；输出目录应位于 `target/`。
- `review_translations.rs`：通用翻译校验与合并工具，从最新提取结果生成 `review.csv`，核对原文、哈希、重复键、占位符及字体覆盖，再合并审定译文。无逐条写死的翻译清单，详见根目录 [AGENTS.md](../AGENTS.md) 的审校流程。
- `subset_font.rs`：生成并验证中文字体子集，替代历史 Python 实现。
- `verify_discovery.rs`：只读比对两次构建的文件集合及逐字节内容，校验重新提取的 CSV 完整保留基准记录，并报告新增候选数。可额外要求当前翻译库与基准一致；重复键、缺列、空基准、丢失或修改记录均会失败。
- `archive/`：以往调查、修补脚本及源码快照，见[历史代码说明](./archive/README.md)。

自动发现改造的文件级回归验证示例（需要先保留基准数据并生成新版输出）：

```powershell
cargo +nightly -Zscript scripts/verify_discovery.rs --before-build target/computer-ui/encode/build --after-build target/auto-discovery/encode/build --baseline-csv target/auto-discovery/baseline.csv --decoded-csv target/auto-discovery/decoded.csv --current-csv translations.csv
```

工具不写死文件数量或本机路径，仅读取传入的数据。此比对不能代替游戏内验证。
