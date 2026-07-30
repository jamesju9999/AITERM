#!/usr/bin/env bash
# Fetch the uv binary used as AITerm's Python environment manager (Linux).
# Run once from the workspace root: ARCH=x64 bash scripts/setup-uv-linux.sh
set -euo pipefail

UV_VERSION="0.11.19"
ARCH="${ARCH:-x64}"

case "$ARCH" in
  x64)   TRIPLE="x86_64-unknown-linux-gnu" ;;
  arm64) TRIPLE="aarch64-unknown-linux-gnu" ;;
  *) echo "ERROR: unsupported ARCH=$ARCH (use x64 or arm64)"; exit 1 ;;
esac

DEST="src-tauri/binaries"
mkdir -p "$DEST"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

URL="https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/uv-${TRIPLE}.tar.gz"
echo "==> Downloading uv ${UV_VERSION} (${TRIPLE})"
curl -L --fail "$URL" -o "$TMP/uv.tar.gz"
tar -xzf "$TMP/uv.tar.gz" -C "$TMP"

cp "$TMP/uv-${TRIPLE}/uv" "$DEST/uv-${TRIPLE}"
chmod +x "$DEST/uv-${TRIPLE}"
echo "==> Wrote $DEST/uv-${TRIPLE}"
