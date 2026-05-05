#!/usr/bin/env bash
# Setup DB2 sidecar for macOS development (Apple Silicon)
# Run once from the workspace root: bash scripts/setup-db2-mac.sh

set -euo pipefail

DEST="src-tauri/binaries/db2-sidecar-mac-arm64"
CLIDRIVER_URL="https://public.dhe.ibm.com/ibmdl/export/pub/software/data/db2/drivers/odbc_cli/macos64_odbc_cli.tar.gz"

echo "==> Creating output directory: $DEST"
mkdir -p "$DEST"

echo "==> Building db2-sidecar for osx-arm64..."
(cd db2-sidecar && dotnet publish -r osx-arm64 --self-contained \
  -o "../$DEST" \
  -p:PublishSingleFile=true \
  --nologo -v quiet)
test -f "$DEST/db2-sidecar" || { echo "ERROR: dotnet publish produced no binary at $DEST/db2-sidecar"; exit 1; }

echo "==> Downloading IBM macOS clidriver (~100MB)..."
TMP=$(mktemp -d)
curl -L "$CLIDRIVER_URL" -o "$TMP/macos64_odbc_cli.tar.gz"

echo "==> Extracting clidriver..."
tar -xzf "$TMP/macos64_odbc_cli.tar.gz" -C "$TMP"
# IBM extracts as "clidriver/" at the top level
cp -R "$TMP/clidriver" "$DEST/clidriver"
rm -rf "$TMP"

echo "==> Making sidecar binary executable..."
chmod +x "$DEST/db2-sidecar"

echo ""
echo "Done. DB2 sidecar ready at: $DEST/"
echo "  db2-sidecar       (binary)"
echo "  clidriver/        (IBM dylibs)"
echo ""
echo "Run 'npm run tauri:dev' to start the app."
