#!/usr/bin/env bash
# 一键打包 release：前端构建 -> Rust 构建（必须带 custom-protocol）-> 复制成带版本号的文件名。
#
# 版本号来源：src-tauri/src/about.rs 的 APP_VERSION（如 26.11_2）。
# exe 的文件属性来自 Cargo.toml 的 version（26.11.2，资源格式写不了下划线），
# 两者由 cargo test 的 version_matches_cargo 守着，改版本时两处一起改。
#
# 用法：bash scripts/pack.sh
# 产物：release/lyric-board_<版本>.exe
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
OUT="release/lyric-board_${VER}.exe"
cp src-tauri/target/release/lyric-board.exe "$OUT"
rm -f release/lyric-board.exe
echo "完成：$OUT"
