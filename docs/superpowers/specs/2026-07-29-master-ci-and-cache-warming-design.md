# master CI 與快取暖機

**日期**：2026-07-29
**狀態**：待審閱

## 背景

### 每次發版都從零開始編譯

v1.2.5 發版時，Rust 快取步驟輸出 `No cache found.`，接著 cargo 下載全部 crates 並完整重編，單一 leg 的建置步驟耗時 508 秒。

原因不是金鑰失效。快取清單顯示三個版本的金鑰**完全相同**：

```
refs/heads/refs/tags/v1.2.3   664MB   v0-rust-linux-x64-appimage-Linux-x64-db7c195c-439e96dc
refs/heads/refs/tags/v1.2.4   664MB   v0-rust-linux-x64-appimage-Linux-x64-db7c195c-439e96dc
refs/heads/refs/tags/v1.2.5   664MB   v0-rust-linux-x64-appimage-Linux-x64-db7c195c-439e96dc
```

差別在 ref。GitHub Actions 的快取依 ref 隔離：一次執行只能還原「同一個 ref」或「預設分支」建立的快取。而 `release.yml` **只在 push tag 時觸發**，每次發版把快取寫進自己那個 tag 的 ref，下一個 tag 結構上不可能讀到。

目前累積 25 份、10.06 GB，正好卡在 GitHub 每 repo 的 10 GB 上限，而這些快取從未被使用過一次。

### 沒有任何測試把關

`release.yml` 是這個 repo 唯一的 workflow，且只在 tag push 時執行。`tsc -b`、Vitest、`cargo test`、Python unittest 全都只在本機執行。

v1.2.4 的六條腿曾全部失敗於一個測試檔的型別錯誤，因為當時記載的檢查指令是 `npx tsc --noEmit`——根 `tsconfig.json` 是 solution 檔（`"files": []`），該指令什麼都不檢查且永遠 exit 0。

**目標**：在預設分支建立可被 tag 執行還原的快取，並在推 tag 之前擋下這類錯誤。

## 設計

新增 `.github/workflows/ci.yml`，兩個互相獨立的 job。

### `test`

| 項目 | 值 |
|---|---|
| 觸發 | push 到 master、pull request |
| runner | ubuntu-latest |
| 預期耗時 | 約 2 分鐘 |

步驟：`npm install` → `npx tsc -b` → `npm run test` → `python3 -m unittest discover -s scripts -p 'test_*.py'`

**必須是 `tsc -b`**，不是 `tsc --noEmit`。這是本 job 存在的直接理由。

不跑 `npm run lint`——既有約 181 個與此無關的問題。不跑 `cargo test`：那需要 Linux 系統相依套件與數分鐘編譯，會讓這個 job 從「快速把關」變成「又一個慢 job」。Rust 的編譯錯誤由 `warm-cache` 覆蓋。

### `warm-cache`

| 項目 | 值 |
|---|---|
| 觸發 | push 到 master、每週排程、手動 dispatch |
| runner | 六個，逐一對應 `release.yml` 的 matrix |
| 預期耗時 | 每個約 8 分鐘，平行 |

必須與 `release.yml` **逐項對齊**，否則金鑰不同、暖了也無效：

| 對齊項 | 理由 |
|---|---|
| runner OS | 金鑰含 `Linux-x64` / `Darwin-arm64` 等 |
| `shared-key` = `artifact_name` | 六條腿刻意隔離（`release.yml` 註解：避免 E0463） |
| **`--release` profile** | release 以 `--release` 建置；debug 產物無法重用 |

矩陣：

| os | target | shared-key |
|---|---|---|
| macos-latest | aarch64-apple-darwin | mac |
| windows-latest | x86_64-pc-windows-msvc | windows |
| ubuntu-22.04 | x86_64-unknown-linux-gnu | linux-x64-appimage |
| ubuntu-24.04 | x86_64-unknown-linux-gnu | linux-x64-deb |
| ubuntu-22.04-arm | aarch64-unknown-linux-gnu | linux-arm64-appimage |
| ubuntu-24.04-arm | aarch64-unknown-linux-gnu | linux-arm64-deb |

步驟：checkout → setup-node → `npm install` → `npm run build` → rust toolchain（含 target）→ rust-cache（對應 shared-key）→ Linux 裝系統相依 → `cargo build --release --target <target>`

跑 `cargo build` 而非完整 `tauri build`：省去打包與簽章，但依賴編譯（耗時主體）一樣進入快取。

**不建 Java sidecar。** 已實測：把 `src-tauri/binaries/` 移走後 `cargo check` 仍 exit 0，代表 `tauri-build` 不驗證 `externalBin`。省去 setup-java 與 Maven。

**但該實測只涵蓋 macOS，結論不可推廣到 Windows。** `tauri.windows.conf.json` 把 sidecar 列在 `bundle.resources`（而非 `externalBin`），而 `tauri-build` **會**驗證 resources 是否存在。首次執行時六個 leg 中只有 Windows 失敗於 `resource path binaries\db2-sidecar-win-x64 doesn't exist`。

Windows leg 因此先建立一個佔位檔。此 job 從不打包，resources 僅被檢查存在性，而本快取要暖的相依編譯與該檔內容無關。`release.yml` 仍以 `scripts/setup-db2-win.ps1` 建置真正的 sidecar，**發版流程完全未改動**。

佔位檔傳不進發版，兩項證據：rust-cache 的 Cache Paths 只有 `~/.cargo/*` 與 `src-tauri/target`，不含 `src-tauri/binaries/`；且該目錄在 `.gitignore:42`。

**已於 v1.2.6 實際確認**：Windows 安裝後 `db2-sidecar` 的 JAR 檔仍存在且完好。佔位檔未污染發版產物，與上述兩項證據一致。

（此項刻意不以推論結案，因為本設計已因「在 macOS 實測後推廣到 Windows」錯過一次。）

排程存在的理由：GitHub 的快取 **7 天無存取即淘汰**。若兩次發版間隔超過七天且 master 無提交，暖好的快取會消失。

## 本設計賴以成立的假設

**tag 觸發的執行能否還原 master 建立的快取，尚未經實證。**

GitHub 文件稱執行可還原「目前分支或預設分支」的快取，但本 repo 的 tag 執行其快取 ref 被記為 `refs/heads/refs/tags/v1.2.5` 這種非典型格式。此假設**無法在發版前驗證**。

因此驗收方式是：下次發版時檢查建置 log 是否出現 `Restored from cache`（而非 `No cache found.`），並比對建置步驟耗時是否明顯低於 508 秒。

## 刻意不做的事

**本次不修改 `release.yml`。**

原本應順手加上 `save-if: false`，讓發版停止寫入那些永遠無人讀取的 3.8 GB。但若上述假設不成立，該改動只會讓「同一個 tag 重跑」失去唯一的快取好處，得不償失。

先驗證，再優化。快取淘汰為 LRU，而 tag 的快取從未被讀取、master 的每次提交都會讀取，因此淘汰壓力自然落在無用的那些上，暫時不改不會惡化。

## 測試策略

CI workflow 本身無法在本機執行。可事前驗證的僅有：

| 項目 | 方法 |
|---|---|
| YAML 可解析 | `yaml.safe_load` |
| 矩陣與 `release.yml` 對齊 | 以程式比對兩份 workflow 的 os / target / shared-key 三元組，逐項相等 |
| `test` job 的指令在本機為綠 | 直接執行該四條指令 |

**矩陣對齊必須用程式比對而非肉眼。** 六組三元組中任何一組寫錯，該 leg 就永遠暖不到，而症狀是「發版時只有部分 leg 變快」——極易被誤判為正常波動。

## 驗證限制

`warm-cache` 是否真的縮短發版時間，只有在下次發版時才知道。這與快取跨 ref 的假設是同一件事。

第一次 push 到 master 時 `warm-cache` 必然是 `No cache found.`（尚無 master 快取可還原），這是預期行為而非失敗；第二次 push 才應出現還原。

## 驗證結果（master 內部，2026-07-29）

第二次推送時五個 leg 皆命中，且為完全匹配：

```
Restored from cache key "v0-rust-linux-x64-appimage-Linux-x64-db7c195c-439e96dc" full match: true.
Restored from cache key "v0-rust-mac-Darwin-arm64-af5aa912-439e96dc" full match: true.
```

建置步驟耗時：

| leg | 冷編 | 命中後 |
|---|---|---|
| linux-x64-appimage | 473s | **85s** |
| linux-arm64-deb | 413s | **87s** |
| linux-x64-deb | 454s | **97s** |
| linux-arm64-appimage | 373s | **103s** |
| mac | 508s | **150s** |
| windows | 488s（首次成功，寫入快取） | 尚未量測 |

約五倍。六份 master 快取皆已就位（含修正後的 Windows）。

`test` job 耗時 60 秒。

**這只證明快取機制在 master 內部有效，不證明 tag 執行讀得到。** 後者於 v1.2.6 驗證，見下。

## 核心假設驗證結果（v1.2.6，2026-07-29）

**假設成立：tag 觸發的執行能還原預設分支建立的快取。** 六條腿全數命中，取自完成後的 build log：

```
linux-x64-appimage     Restored from cache key "v0-rust-linux-x64-appimage-Linux-x64-db…"
linux-x64-deb          Restored from cache key "v0-rust-linux-x64-deb-Linux-x64-db…"
linux-arm64-appimage   Restored from cache key "v0-rust-linux-arm64-appimage-Linux-arm64-…"
linux-arm64-deb        Restored from cache key "v0-rust-linux-arm64-deb-Linux-arm64-27a…"
mac                    Restored from cache key "v0-rust-mac-Darwin-arm64-af5aa912-439e9…"
windows                Restored from cache key "v0-rust-windows-Windows_NT-x64-8af1e26a…"
```

步驟耗時對照（同一條 linux-x64-appimage）：

| 步驟 | v1.2.5（`No cache found.`） | v1.2.6 |
|---|---|---|
| `Rust cache` | **0 秒** | **15 秒** |
| `Build` | **508 秒** | **271 秒** |

`Rust cache` 耗時 0 秒代表無物可還原；15 秒代表確實下載並解開了約 660 MB。這在 log 出現之前就是強力指標，兩者結論一致。

v1.2.6 六條腿的 Build 耗時：127s（arm64-deb）、144s（arm64-appimage）、233s（x64-deb）、271s（x64-appimage）、323s（mac）、390s（windows）。

較 `warm-cache` 的 85 秒為長，是因為發版還要打包、執行 linuxdeploy、產生 AppImage，那些 `cargo build` 不做。

### 後續優化現已解鎖

假設既已成立，`release.yml` 的 rust-cache 可加上 `save-if: false`，停止每次發版寫入約 3.8 GB 永遠無人讀取的快取。**尚未執行。**

快取總量目前 10.06 GB，其中 15 份仍屬舊 tag。由於 LRU 淘汰且 tag 快取從未被讀取，這些會優先被清除。
