# Fetch the uv binary used as AITerm's Python environment manager (Windows x64).
# Run once from the workspace root:
#   powershell -ExecutionPolicy Bypass -File scripts\setup-uv-win.ps1

$ErrorActionPreference = "Stop"

$UV_VERSION = "0.11.19"
$TRIPLE = "x86_64-pc-windows-msvc"
$DEST = "src-tauri\binaries"

New-Item $DEST -ItemType Directory -Force | Out-Null

$TMP = New-Item -ItemType Directory -Path ([System.IO.Path]::GetTempPath()) -Name ([guid]::NewGuid())
try {
  $URL = "https://github.com/astral-sh/uv/releases/download/$UV_VERSION/uv-$TRIPLE.zip"
  Write-Host "==> Downloading uv $UV_VERSION ($TRIPLE)"
  Invoke-WebRequest -Uri $URL -OutFile "$TMP\uv.zip"
  Expand-Archive -Path "$TMP\uv.zip" -DestinationPath $TMP -Force

  # Tauri's externalBin requires the target-triple suffix on the filename.
  Copy-Item "$TMP\uv.exe" "$DEST\uv-$TRIPLE.exe" -Force
  Write-Host "==> Wrote $DEST\uv-$TRIPLE.exe"
  & "$DEST\uv-$TRIPLE.exe" --version
} finally {
  Remove-Item $TMP -Recurse -Force
}
