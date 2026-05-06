#!/usr/bin/env bash
# Setup DB2 Java sidecar for macOS (Apple Silicon ARM64)
# Run once from the workspace root: bash scripts/setup-db2-mac.sh

set -euo pipefail

DEST="src-tauri/binaries/db2-sidecar-mac-arm64"

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

echo "==> Downloading Eclipse Temurin 21 JRE (macOS ARM64)..."
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

JRE_URL="https://api.adoptium.net/v3/binary/latest/21/ga/mac/aarch64/jre/hotspot/normal/eclipse"
curl -L --fail "$JRE_URL" -o "$TMP/jre.tar.gz"

echo "==> Extracting JRE..."
tar -xzf "$TMP/jre.tar.gz" -C "$TMP"

# Temurin extracts as jdk-21.*-jre/ — find the extracted directory
JRE_DIR=$(find "$TMP" -maxdepth 1 -name "jdk-*-jre" -type d | head -1)
if [[ -z "$JRE_DIR" ]]; then
  # Some builds extract as a .jre directory
  JRE_DIR=$(find "$TMP" -maxdepth 1 -name "*.jre" -type d | head -1)
fi
if [[ -z "$JRE_DIR" ]]; then
  echo "ERROR: Could not locate extracted JRE directory in $TMP"
  ls "$TMP"
  exit 1
fi

# On macOS, Temurin JRE contains a Contents/Home structure
if [[ -d "$JRE_DIR/Contents/Home" ]]; then
  JRE_HOME="$JRE_DIR/Contents/Home"
else
  JRE_HOME="$JRE_DIR"
fi

mkdir -p "$DEST/jre"
cp -R "$JRE_HOME/." "$DEST/jre/"

echo "==> Verifying java binary..."
"$DEST/jre/bin/java" -version 2>&1 | head -1

echo ""
echo "Done. DB2 Java sidecar ready at: $DEST/"
echo "  db2sidecar.jar          (IBM JDBC fat jar)"
echo "  jre/bin/java            (Temurin 21 ARM64)"
echo ""
echo "Run 'npm run tauri:dev' to start the app."
