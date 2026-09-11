# AITerm CLI Host 安裝腳本（Windows）。
#
#   irm https://raw.githubusercontent.com/jamesju9999/AITERM/master/scripts/install.ps1 | iex
#
# 跟 install.sh 同樣的四步：偵測 → 下載 → 驗 checksum → 安裝。
# **checksum 不通過就中止**，不給略過的選項。
$ErrorActionPreference = "Stop"

$Repo = "jamesju9999/AITERM"
$InstallDir = if ($env:AITERM_HOST_INSTALL_DIR) { $env:AITERM_HOST_INSTALL_DIR } else { "$env:LOCALAPPDATA\Programs\aiterm-host" }

if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne "X64") {
    throw "目前只提供 x86_64 的 Windows 執行檔，偵測到：$([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture)"
}
$Target = "x86_64-pc-windows-msvc"

Write-Host "正在查最新版本…"
$release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
$version = $release.tag_name -replace '^v', ''
Write-Host "最新版本：$version"

$name = "aiterm-host-$version-$Target"
$base = "https://github.com/$Repo/releases/download/v$version"
$tmp = New-Item -ItemType Directory -Path (Join-Path $env:TEMP ([System.Guid]::NewGuid()))

try {
    Write-Host "下載 $name.zip…"
    Invoke-WebRequest "$base/$name.zip" -OutFile "$tmp\$name.zip"
    Invoke-WebRequest "$base/aiterm-host-$version-SHA256SUMS" -OutFile "$tmp\SHA256SUMS"

    Write-Host "驗證 checksum…"
    $expected = (Get-Content "$tmp\SHA256SUMS" | Where-Object { $_ -match [regex]::Escape("$name.zip") })
    if (-not $expected) { throw "SHA256SUMS 裡找不到 $name.zip 的項目——中止。" }
    $expectedHash = ($expected -split '\s+')[0].ToLower()
    $actualHash = (Get-FileHash "$tmp\$name.zip" -Algorithm SHA256).Hash.ToLower()
    if ($expectedHash -ne $actualHash) {
        throw "checksum 不符——中止安裝。下載的檔案可能被竄改或損毀。"
    }

    Expand-Archive "$tmp\$name.zip" -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    Copy-Item "$tmp\$name\aiterm-host.exe" "$InstallDir\aiterm-host.exe" -Force

    Write-Host "已安裝：$InstallDir\aiterm-host.exe"
    if ($env:PATH -notlike "*$InstallDir*") {
        Write-Host "提醒：$InstallDir 不在 PATH 裡。"
    }
    Write-Host "下一步：aiterm-host --print-connection"
}
finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
