#!/usr/bin/env bash
# 一键打包 release：前端构建 -> Rust 构建（必须带 custom-protocol）-> 复制成带版本号的文件名。
#
# 版本号来源：src-tauri/src/about.rs 的 APP_VERSION（如 26.11_2）。
# exe 的文件属性来自 Cargo.toml 的 version（26.11.2，资源格式写不了下划线），
# 两者由 cargo test 的 version_matches_cargo 守着，改版本时两处一起改。
#
# 用法：bash scripts/pack.sh
# 产物：release/LyricsFloat_<版本>.exe
set -euo pipefail
cd "$(dirname "$0")/.."

VER=$(sed -n 's/.*APP_VERSION: &str = "\([^"]*\)".*/\1/p' src-tauri/src/about.rs | head -1)
if [ -z "$VER" ]; then
  echo "读不到 APP_VERSION，检查 src-tauri/src/about.rs" >&2
  exit 1
fi
echo "==> 目标版本 $VER"

echo "==> 1/3 前端构建"
npx vite build

echo "==> 2/3 Rust release 构建（带 custom-protocol）"
# shellcheck disable=SC1091
source scripts/msvc-env.sh
cargo build --release --features custom-protocol --manifest-path src-tauri/Cargo.toml

echo "==> 3/3 复制产物"
mkdir -p release
# 产物名固定 LyricsFloat.exe（不再带版本号）：Release 资产名稳定，
# README 的 /releases/latest/download/LyricsFloat.exe 才能真正"直接下载最新版"。
# 版本号照样写在 exe 的文件属性里（ProductVersion / FileVersion，来自 Cargo.toml），
# 右键属性就能看到，也能被自动更新逻辑读到。
OUT="release/LyricsFloat.exe"
cp src-tauri/target/release/lyricsfloat.exe "$OUT"
# 注意：Windows 文件名大小写不敏感，这里**不要**再 rm 小写的 lyricsfloat.exe ——
# 那就是刚复制出来的这个文件，会把产物删掉。只清历史遗留的旧名字。
rm -f release/lyric-board.exe
# 清掉旧版留下的带版本号产物，免得 release 里堆一排 exe
find release -maxdepth 1 -name '*.exe' ! -name "$(basename "$OUT")" -delete 2>/dev/null || true
echo "完成：$OUT（版本 $VER 已写进文件属性）"
