set shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

script := "scripts/nuclear_cn.rs"
default_csv := "origin.csv"
default_pak := "NuclearNightmare/Content/Paks/zz_NuclearNightmare_Chinese_P.pak"

# 显示可用命令
default:
    @just --list

# 从游戏 PAK 提取待翻译的 CSV
decode output=default_csv:
    cargo +nightly -Zscript "{{script}}" decode --output "{{output}}"

# 将填写完成的 CSV 严格校验并编译为汉化 PAK
encode input=default_csv output=default_pak:
    cargo +nightly -Zscript "{{script}}" encode --input "{{input}}" --output "{{output}}" --strict

# 允许 trans_text 留空；空白项会回退为英文原文
encode-loose input=default_csv output=default_pak:
    cargo +nightly -Zscript "{{script}}" encode --input "{{input}}" --output "{{output}}"

# 查看 Rust CLI 自带帮助
help:
    cargo +nightly -Zscript "{{script}}" --help

# 生成并验证常用中文和 UI 字体子集（输出到 target/font-subsets）
subset-font output="target/font-subsets":
    cargo +nightly -Zscript scripts/subset_font.rs --output-dir "{{output}}"
