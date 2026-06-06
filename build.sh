#!/usr/bin/env bash
# statusline-rs ビルドスクリプト
# 使い方: bash build.sh
set -euo pipefail

SCRIPT_DIR="$(cd -P "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# PATH に ~/.cargo/bin を追加（rustup 環境）
export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v cargo >/dev/null 2>&1; then
  echo "ERROR: cargo が見つかりません。以下でインストールしてください:"
  echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  exit 1
fi

echo "Building statusline-rs (release)..."
cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"

echo "Done: $SCRIPT_DIR/target/release/statusline-rs"
