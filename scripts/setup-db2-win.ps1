# Setup DB2 Java sidecar for Windows x64
# Run once from the workspace root:
#   powershell -ExecutionPolicy Bypass -File scripts\setup-db2-win.ps1

$ErrorActionPreference = "Stop"

$DEST = "src-tauri\binaries\db2-sidecar-win-x64"

Write-Host "==> Cleaning output directory: $DEST"
if (Test-Path $DEST) { Remove-Item $DEST -Recurse -Force }
New-Item $DEST -ItemType Directory | Out-Null

Write-Host "==> Building db2sidecar.jar via Maven..."
Push-Location "db2-sidecar-java"
mvn package -q --no-transfer-progress
Pop-Location

$JAR = "db2-sidecar-java\target\db2sidecar.jar"
if (-not (Test-Path $JAR)) {
    Write-Error "ERROR: mvn package produced no jar at $JAR"
    exit 1
}
Copy-Item $JAR "$DEST\db2sidecar.jar"
Write-Host "  Copied db2sidecar.jar"

Write-Host "==> Downloading Eclipse Temurin 21 JRE (Windows x64)..."
$TMP = New-TemporaryFile | ForEach-Object { Remove-Item $_; New-Item -ItemType Directory $_.FullName }
$JRE_URL = "https://api.adoptium.net/v3/binary/latest/21/ga/windows/x64/jre/hotspot/normal/eclipse"
$JRE_ZIP = "$TMP\jre.zip"
Invoke-WebRequest -Uri $JRE_URL -OutFile $JRE_ZIP -UseBasicParsing

Write-Host "==> Extracting JRE..."
Expand-Archive -Path $JRE_ZIP -DestinationPath $TMP

# Temurin extracts as jdk-21.*-jre\ on Windows
$JRE_DIR = Get-ChildItem $TMP -Directory | Where-Object { $_.Name -match "jdk-.*-jre" } | Select-Object -First 1
if (-not $JRE_DIR) {
    Write-Error "ERROR: Could not locate extracted JRE directory in $TMP"
    Get-ChildItem $TMP
    exit 1
}

New-Item "$DEST\jre" -ItemType Directory | Out-Null
Copy-Item "$($JRE_DIR.FullName)\*" "$DEST\jre\" -Recurse

Write-Host "==> Verifying java.exe..."
& "$DEST\jre\bin\java.exe" -version

Remove-Item $TMP -Recurse -Force

Write-Host ""
Write-Host "Done. DB2 Java sidecar ready at: $DEST\"
Write-Host "  db2sidecar.jar          (IBM JDBC fat jar)"
Write-Host "  jre\bin\java.exe        (Temurin 21 x64)"
Write-Host ""
Write-Host "Run 'npm run tauri:dev' to start the app."
