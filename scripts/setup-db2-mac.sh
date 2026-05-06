#!/usr/bin/env bash
# Setup DB2 sidecar for macOS development (Apple Silicon)
# Run once from the workspace root: bash scripts/setup-db2-mac.sh

set -euo pipefail

DEST="src-tauri/binaries/db2-sidecar-mac-arm64"
CLIDRIVER_URL="https://public.dhe.ibm.com/ibmdl/export/pub/software/data/db2/drivers/odbc_cli/macos64_odbc_cli.tar.gz"

echo "==> Cleaning output directory: $DEST"
rm -rf "$DEST"
mkdir -p "$DEST"

echo "==> Building db2-sidecar for osx-x64 (IBM clidriver is x86_64; runs via Rosetta 2 on Apple Silicon)..."
(cd db2-sidecar && dotnet publish -r osx-x64 --self-contained \
  -o "../$DEST" \
  -p:PublishSingleFile=true \
  --nologo -v quiet)
test -f "$DEST/db2-sidecar" || { echo "ERROR: dotnet publish produced no binary at $DEST/db2-sidecar"; exit 1; }

echo "==> Downloading IBM macOS clidriver (~100MB)..."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -L "$CLIDRIVER_URL" -o "$TMP/macos64_odbc_cli.tar.gz"

echo "==> Extracting clidriver..."
tar -xzf "$TMP/macos64_odbc_cli.tar.gz" -C "$DEST"
# The tarball extracts directly as clidriver/ containing the macOS dylibs.

echo "==> Fixing permissions (IBM clidriver ships r-xr-xr-x; Tauri build needs u+rw to overwrite)..."
chmod -R u+rw "$DEST/clidriver"
chmod +x "$DEST/db2-sidecar"

# ── GCC x86_64 runtime ───────────────────────────────────────────────────────
# libdb2.dylib was compiled with GCC 8 and hardlinks against:
#   /usr/local/lib/gcc/8/libstdc++.6.dylib
#   /usr/local/lib/gcc/8/libgcc_s.1.dylib
# These are not present on modern macOS.  We fetch the x86_64 GCC bottle from
# Homebrew (arm64 brew can fetch x86_64 bottles), extract the runtime dylibs,
# copy them into clidriver/lib/, and patch libdb2.dylib to use @loader_path.

echo "==> Fetching GCC x86_64 runtime from Homebrew (needed by IBM libdb2)..."

# Pick bottle tag matching this macOS major version
OS_MAJOR=$(sw_vers -productVersion | cut -d. -f1)
case "$OS_MAJOR" in
  15) GCC_BOTTLE_TAG="sequoia" ;;
  14) GCC_BOTTLE_TAG="sonoma"  ;;
  13) GCC_BOTTLE_TAG="ventura" ;;
  *)  GCC_BOTTLE_TAG="sequoia" ;;
esac

BREW_CACHE="$(brew --cache)/downloads"

# Download if the bottle isn't already cached
if ! ls "$BREW_CACHE"/*gcc*"${GCC_BOTTLE_TAG}".bottle.tar.gz 2>/dev/null | grep -q .; then
  brew fetch --bottle-tag="$GCC_BOTTLE_TAG" gcc 2>&1 | grep -v "^==>.*Already downloaded"
fi

GCC_BOTTLE=$(ls "$BREW_CACHE"/*gcc*"${GCC_BOTTLE_TAG}".bottle.tar.gz 2>/dev/null | head -1)
if [[ -z "$GCC_BOTTLE" ]]; then
  echo "ERROR: Could not find GCC bottle after brew fetch."
  echo "Try manually: brew fetch --bottle-tag=${GCC_BOTTLE_TAG} gcc"
  exit 1
fi

echo "==> Extracting GCC runtime libraries from bottle..."
GCC_TMP=$(mktemp -d)
tar -xzf "$GCC_BOTTLE" -C "$GCC_TMP"

LIBSTDCXX=$(find "$GCC_TMP" -name "libstdc++.6.dylib" | head -1)
LIBGCC11=$(find "$GCC_TMP" -name "libgcc_s.1.1.dylib" | head -1)
LIBGCC1=$(find "$GCC_TMP" -name "libgcc_s.1.dylib" ! -type l | head -1)
# Prefer 1.1 (GCC 13+), fall back to 1 (older GCC)
LIBGCC="${LIBGCC11:-$LIBGCC1}"

if [[ -z "$LIBSTDCXX" || -z "$LIBGCC" ]]; then
  echo "ERROR: Could not locate libstdc++ / libgcc_s in the GCC bottle."
  rm -rf "$GCC_TMP"
  exit 1
fi

CLILIB="$DEST/clidriver/lib"

cp "$LIBSTDCXX" "$CLILIB/libstdc++.6.dylib"
cp "$LIBGCC"    "$CLILIB/libgcc_s.1.1.dylib"
# GCC bottle files ship read-only; Tauri build copies resources via fs::copy
# which fails on read-only destinations — make them writable.
chmod u+rw "$CLILIB/libstdc++.6.dylib" "$CLILIB/libgcc_s.1.1.dylib"
# libdb2.dylib (GCC 8) expects libgcc_s.1.dylib; point it at the .1.1 copy
ln -sf libgcc_s.1.1.dylib "$CLILIB/libgcc_s.1.dylib"
rm -rf "$GCC_TMP"

echo "==> Patching GCC runtime LC_IDs to @loader_path..."
# Homebrew bottles embed @@HOMEBREW_PREFIX@@ placeholders; fix them
HBPREFIX="@@HOMEBREW_PREFIX@@"
install_name_tool \
  -id "@loader_path/libstdc++.6.dylib" \
  -change "${HBPREFIX}/opt/gcc/lib/gcc/current/libstdc++.6.dylib" "@loader_path/libstdc++.6.dylib" \
  "$CLILIB/libstdc++.6.dylib"

install_name_tool \
  -id "@loader_path/libgcc_s.1.1.dylib" \
  "$CLILIB/libgcc_s.1.1.dylib"

echo "==> Patching libdb2.dylib GCC 8 deps to use bundled @loader_path copies..."
LIBDB2="$CLILIB/libdb2.dylib"
install_name_tool \
  -change "/usr/local/lib/gcc/8/libstdc++.6.dylib" "@loader_path/libstdc++.6.dylib" \
  -change "/usr/local/lib/gcc/8/libgcc_s.1.dylib"  "@loader_path/libgcc_s.1.dylib"  \
  "$LIBDB2"

# ── IBM.Data.Db2.Core P/Invoke shim ──────────────────────────────────────────
# IBM.Data.Db2.Core v3 hard-codes [DllImport("db2app64.dll")].
# On macOS, .NET resolves this as "db2app64.dll.dylib".
# The macOS clidriver exposes the same symbols via libdb2.dylib.
echo "==> Creating db2app64.dll.dylib → libdb2.dylib symlink..."
ln -sf libdb2.dylib "$CLILIB/db2app64.dll.dylib"

echo ""
echo "Done. DB2 sidecar ready at: $DEST/"
echo "  db2-sidecar              (binary, osx-x64 via Rosetta 2)"
echo "  clidriver/lib/libdb2.dylib          (IBM CLI driver)"
echo "  clidriver/lib/libstdc++.6.dylib     (GCC x86_64 runtime)"
echo "  clidriver/lib/libgcc_s.1.1.dylib    (GCC x86_64 runtime)"
echo "  clidriver/lib/db2app64.dll.dylib    (symlink → libdb2.dylib)"
echo ""
echo "Run 'npm run tauri:dev' to start the app."
