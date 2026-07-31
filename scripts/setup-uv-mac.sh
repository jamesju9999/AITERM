#!/usr/bin/env bash
# Fetch the uv binary used as AITerm's Python environment manager (macOS).
# Run once from the workspace root: bash scripts/setup-uv-mac.sh
set -euo pipefail

UV_VERSION="0.11.19"

if [ "$(uname -m)" = "arm64" ]; then
  TRIPLE="aarch64-apple-darwin"
else
  TRIPLE="x86_64-apple-darwin"
fi

DEST="src-tauri/binaries"
mkdir -p "$DEST"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

URL="https://github.com/astral-sh/uv/releases/download/${UV_VERSION}/uv-${TRIPLE}.tar.gz"
echo "==> Downloading uv ${UV_VERSION} (${TRIPLE})"
curl -L --fail "$URL" -o "$TMP/uv.tar.gz"
tar -xzf "$TMP/uv.tar.gz" -C "$TMP"

# Tauri's externalBin requires the target-triple suffix on the filename.
cp "$TMP/uv-${TRIPLE}/uv" "$DEST/uv-${TRIPLE}"
chmod +x "$DEST/uv-${TRIPLE}"
echo "==> Wrote $DEST/uv-${TRIPLE}"
"$DEST/uv-${TRIPLE}" --version
