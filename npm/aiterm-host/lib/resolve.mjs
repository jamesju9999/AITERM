// 平台套件一律帶範圍。不帶範圍的 `aiterm-host-win32-x64` 被 npm 的名稱防濫用
// 機制擋下（`403 Package name triggered spam detection`），隔天、不同的 run
// 重試仍然一樣——不是速率限制，是名稱本身被判定有問題（`<name>-<os>-<arch>`
// 這種形狀常被拿來做惡意的名稱佔用）。帶範圍的名稱在自己的命名空間裡，不會
// 觸發那個機制。
//
// **範圍名稱必須是 npm 的帳號名稱，不是 GitHub 的。** 兩邊不同名：GitHub 是
// jamesju9999，npm 是 jamesju。用 @jamesju9999 發佈時 npm 回
// `404 Not Found - PUT ... - Scope not found`——使用者範圍只會自動對應到自己的
// 帳號名，別的名字既不存在也建不出來（要建就得開 organization）。
// 確認方法：`npm view <任何一個既有套件> maintainers`。
const SCOPE = "@jamesju";
const PACKAGES = {
  "darwin arm64": `${SCOPE}/aiterm-host-darwin-arm64`,
  "darwin x64": `${SCOPE}/aiterm-host-darwin-x64`,
  "linux x64": `${SCOPE}/aiterm-host-linux-x64`,
  "linux arm64": `${SCOPE}/aiterm-host-linux-arm64`,
  "win32 x64": `${SCOPE}/aiterm-host-win32-x64`,
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
