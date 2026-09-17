# 編譯 aiterm-elevated-host 並複製到 Tauri externalBin 要求的位置（Windows x64）。
# Run once from the workspace root:
#   powershell -ExecutionPolicy Bypass -File scripts\setup-elevated-host-win.ps1

$ErrorActionPreference = "Stop"

$TRIPLE = "x86_64-pc-windows-msvc"
$DEST = "src-tauri\binaries"

Push-Location src-tauri
try {
  Write-Host "==> Building aiterm-elevated-host (release)"
  cargo build --release -p aiterm-elevated-host
  if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
} finally {
  Pop-Location
}

New-Item $DEST -ItemType Directory -Force | Out-Null
Copy-Item "src-tauri\target\release\aiterm-elevated-host.exe" "$DEST\aiterm-elevated-host-$TRIPLE.exe" -Force
Write-Host "==> Wrote $DEST\aiterm-elevated-host-$TRIPLE.exe"
