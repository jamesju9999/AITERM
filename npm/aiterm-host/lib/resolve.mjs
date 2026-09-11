const PACKAGES = {
  "darwin arm64": "aiterm-host-darwin-arm64",
  "darwin x64": "aiterm-host-darwin-x64",
  "linux x64": "aiterm-host-linux-x64",
  "linux arm64": "aiterm-host-linux-arm64",
  "win32 x64": "aiterm-host-win32-x64",
};

/** 這個平台該用哪個子套件。不支援就丟錯，不回 undefined。 */
export function packageForPlatform(platform, arch) {
  const name = PACKAGES[`${platform} ${arch}`];
  if (!name) {
    throw new Error(
      `aiterm-host 沒有提供 ${platform} ${arch} 的執行檔。` +
        `支援的平台：${Object.keys(PACKAGES).join(", ")}`,
    );
  }
  return name;
}

/** 執行檔的檔名。Windows 要帶 .exe。 */
export function binaryName(platform) {
  return platform === "win32" ? "aiterm-host.exe" : "aiterm-host";
}
