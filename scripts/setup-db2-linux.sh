#!/usr/bin/env bash
# Setup DB2 Java sidecar for Linux (x64 or arm64)
# Usage: bash scripts/setup-db2-linux.sh
# Set ARCH=x64 (default) or ARCH=arm64 to override auto-detection.
# Run from the workspace root.

set -euo pipefail

# Auto-detect architecture if not set
if [[ -z "${ARCH:-}" ]]; then
  case "$(uname -m)" in
    x86_64)  ARCH="x64" ;;
    aarch64) ARCH="arm64" ;;
    *)
      echo "ERROR: Unsupported architecture: $(uname -m). Set ARCH=x64 or ARCH=arm64."
      exit 1
      ;;
  esac
fi

case "$ARCH" in
  x64)
    ADOPTIUM_ARCH="x64"
    DEST="src-tauri/binaries/db2-sidecar-linux-x64"
    ;;
  arm64)
    ADOPTIUM_ARCH="aarch64"
    DEST="src-tauri/binaries/db2-sidecar-linux-arm64"
    ;;
  *)
    echo "ERROR: ARCH must be 'x64' or 'arm64', got: $ARCH"
    exit 1
    ;;
esac

echo "==> Architecture: $ARCH"
echo "==> Cleaning output directory: $DEST"
rm -rf "$DEST"
mkdir -p "$DEST"

echo "==> Building db2sidecar.jar via Maven..."
(cd db2-sidecar-java && mvn package -q --no-transfer-progress)
test -f "db2-sidecar-java/target/db2sidecar.jar" || {
  echo "ERROR: mvn package produced no jar at db2-sidecar-java/target/db2sidecar.jar"
  exit 1
}
cp "db2-sidecar-java/target/db2sidecar.jar" "$DEST/db2sidecar.jar"
echo "  Copied db2sidecar.jar"

echo "==> Downloading Eclipse Temurin 21 JRE (Linux $ARCH)..."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

JRE_URL="https://api.adoptium.net/v3/binary/latest/21/ga/linux/${ADOPTIUM_ARCH}/jre/hotspot/normal/eclipse"
curl -L --fail "$JRE_URL" -o "$TMP/jre.tar.gz"

echo "==> Extracting JRE..."
tar -xzf "$TMP/jre.tar.gz" -C "$TMP"

JRE_DIR=$(find "$TMP" -maxdepth 1 -name "jdk-*-jre" -type d | head -1)
if [[ -z "$JRE_DIR" ]]; then
  JRE_DIR=$(find "$TMP" -maxdepth 1 -name "*.jre" -type d | head -1)
fi
if [[ -z "$JRE_DIR" ]]; then
  echo "ERROR: Could not locate extracted JRE directory in $TMP"
  ls "$TMP"
  exit 1
fi

mkdir -p "$DEST/jre"
cp -R "$JRE_DIR/." "$DEST/jre/"

echo "==> Verifying java binary..."
"$DEST/jre/bin/java" -version 2>&1 | head -1

echo ""
echo "Done. DB2 Java sidecar ready at: $DEST/"
echo "  db2sidecar.jar          (IBM JDBC fat jar)"
echo "  jre/bin/java            (Temurin 21 Linux $ARCH)"
echo ""
echo "Run 'npm run tauri:dev' to start the app."
