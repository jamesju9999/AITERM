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
