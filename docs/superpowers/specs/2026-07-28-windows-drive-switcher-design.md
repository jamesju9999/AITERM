# Windows 磁碟機切換（檔案面板）

**日期**：2026-07-28
**狀態**：待審閱

## 背景與目標

AITerm 的檔案面板（`src/components/FileExplorer/FileExplorer.tsx`）在 Windows 上**無法切換磁碟機**。一旦停在 `C:`，就沒有任何 UI 路徑能到 `D:`。

現況經程式碼查證，三個層面都不支援：

1. **`↑` 按鈕在磁碟機根目錄被停用**——`FileExplorer.tsx:154`：
   ```ts
   const atRoot = !cwd || cwd === "/" || /^[A-Za-z]:\/?\s*$/.test(cwd.replace(/\\/g, "/"));
   ```
   `C:/` 符合此判斷，按鈕 `disabled`。

2. **麵包屑最前面的 `/` 也回不到磁碟機清單**——`FileExplorer.tsx:201-207` 是**從當前路徑**推導磁碟機根目錄（`firstPart + "/"`），在 `C:/Users/…` 底下點它只會回到 `C:/`。

3. **後端沒有列舉磁碟機的能力**——`src-tauri/src/pty/commands.rs:122` 的 `pty_list_dir` 是單純的 `std::fs::read_dir`；全域搜尋 `src-tauri/` 沒有任何磁碟機列舉程式碼。

目前唯一的變通方式是在終端機打 `cd D:\`，讓面板透過 CWD 輪詢跟過去。

**目標**：在工具列提供磁碟機下拉選單，讓使用者直接切換。

## 範圍界定（已與使用者確認）

| 決策點 | 選擇 |
|---|---|
| 互動方式 | **工具列下拉選單**，不動 `↑` / `atRoot` / `goUp` |
| 清單內容 | **僅磁碟機代號**（`C:`、`D:`），不含卷標與剩餘空間 |
| 列舉方式 | **`GetLogicalDrives()`**（單次 Win32 呼叫，不碰磁碟 I/O） |
| 選取行為 | **只切換面板**，不 `cd` 終端機 |
| 麵包屑 `/` | **不動**（見「明確排除」） |

### 明確排除（Non-goals）

- **不修改麵包屑最前面的 `/`。** 使用者截圖中的 `//C:` 來自該 `/` span 加分隔符再加 `C:`；它在 Windows 上沒有語意，但修改它會動到既有導覽行為，屬於獨立議題。
- 不顯示卷標（需 `GetVolumeInformationW`）或剩餘空間（需 `GetDiskFreeSpaceExW`，且網路碟碟可能阻塞數秒）。
- 不修改 `↑` 按鈕在磁碟機根目錄的停用行為，也不新增「本機」這種虛擬層級。
- 不讓選取磁碟機連帶切換終端機工作目錄——工具列已有獨立的「切換終端機到這裡」按鈕負責該職責。
- 不處理 UNC 路徑（`\\server\share`）。

## 為什麼選下拉而非「↑ 再往上」

「`↑` 在 `C:/` 再按一次進入磁碟機清單」比較貼近物件管理員，但代價是 `cwd` 會變成一個**非真實路徑的虛擬層**，而 `cwd` 同時被麵包屑、`focusedPath`、「切換終端機到這裡」與 CWD 輪詢四處消費。下拉選單讓 `cwd` 永遠維持真實路徑，把改動限制在工具列一角。

## 後端設計

`src-tauri/src/pty/commands.rs` 新增：

```rust
#[tauri::command]
pub fn list_drives() -> Vec<String>
```

- **Windows**：呼叫 `GetLogicalDrives()`，回傳 32 位元遮罩（bit 0 = `A:`，bit 1 = `B:`…），轉為 `["C:/", "D:/"]`。
- **非 Windows**：回傳空 `Vec`。

位元運算抽成純函式：

```rust
fn drives_from_mask(mask: u32) -> Vec<String>
```

**這樣拆的理由**：Win32 呼叫在 macOS/Linux 上無法執行，但位元轉字串這段才是真正會寫錯的邏輯（off-by-one、字母對映）。抽成純函式後可在任何平台以單元測試覆蓋，`#[cfg(windows)]` 只包住取得遮罩那一行。這與既有的 `commands/updater.rs::supported_for` 是同一種模式。

依賴加在平台專屬區塊，其他平台完全不編譯：

```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.60", features = ["Win32_Storage_FileSystem"] }
```

`windows-sys` 已存在於 `Cargo.lock`（6 個版本，皆為傳遞依賴），因此加為直接依賴的建置成本可忽略。

路徑格式回傳 `"C:/"`（正斜線），與 `pty_list_dir` 既有的 `norm()` 輸出一致，前端無須再正規化。

## 前端設計

### IPC

`src/ipc/fs.ts` 新增，比照既有 wrapper：

```ts
export const listDrives = (): Promise<string[]> => invoke("list_drives");
```

### UI

`FileExplorer.tsx` 工具列在 `↻` 之後、麵包屑之前，插入一顆顯示目前磁碟機代號的按鈕；點擊展開下拉，列出所有磁碟機。

- 目前磁碟機由 `cwd` 以 `/^([A-Za-z]:)/` 取得。取不到時（理論上不應發生）按鈕顯示清單第一項。
- **少於 2 個磁碟機時整顆按鈕不渲染。** 這一條同時涵蓋兩種情形：非 Windows 平台回傳空陣列，以及 Windows 上只有單一磁碟機——後者沒有任何可切換的對象，而麵包屑已經顯示 `C:`，再放一顆下拉只是雜訊。
- **每次展開下拉時重新抓取清單**，而非僅在 mount 時抓一次——否則插拔 USB 隨身碟後不會反映。單次 Win32 呼叫，成本可忽略。
- 重新抓取失敗時**保留上一次成功取得的清單**，不清空選單。使用者已經點開選單，此時把它變空比顯示可能略舊的清單更糟。

選取磁碟機時：

```ts
loadDir(drive);
setExpanded(new Set());
setSubEntries({});
```

與麵包屑點擊完全相同的三步驟。

## 核心不變式：不得觸碰 `ptyCwdRef`

**選取磁碟機必須只呼叫 `loadDir()`，絕不更新 `ptyCwdRef`。**

`ptyCwdRef`（`FileExplorer.tsx:58`）記錄「最後觀察到的 PTY CWD」，每 1.5 秒的輪詢（`FileExplorer.tsx:101-114`）以它判斷終端機是否自行換過目錄：

```ts
if (newCwd && newCwd !== ptyCwdRef.current) { ptyCwdRef.current = newCwd; loadDir(newCwd); }
```

若選取磁碟機時一併更新 `ptyCwdRef`，輪詢會誤以為終端機已在 `D:`；之後終端機真的 `cd` 時，比對結果可能相等而不觸發重載，**檔案面板將從此不再跟隨終端機**。

行為結果：切到 `D:` 後面板停在 `D:`；待終端機自行 `cd` 時才跳回跟隨。這與現行麵包屑導覽的行為一致。

## 錯誤處理

| 情境 | 行為 |
|---|---|
| `list_drives` 首次呼叫失敗 | 視同空清單，按鈕不渲染（功能靜默降級，不阻斷檔案面板） |
| 展開下拉時重新抓取失敗 | 保留上一次成功的清單，選單不清空 |
| 選取的磁碟機已移除 | `loadDir` 走既有錯誤路徑，於面板顯示錯誤訊息 |
| 非 Windows 平台 | 回傳空清單，UI 不出現 |

## 測試策略

| 層級 | 內容 |
|---|---|
| Rust 單元測試 | `drives_from_mask`：空遮罩、僅 `A:`、`C:`+`D:`、全 26 個、最高位元 |
| 前端 | 空清單（非 Windows）時按鈕不渲染 |
| 前端 | 只有單一磁碟機時按鈕不渲染 |
| 前端 | 有清單時按鈕渲染，且顯示由 `cwd` 推導的目前磁碟機 |
| 前端 | 選取磁碟機後，`listDirectory` 收到該磁碟機根路徑 |
| 前端 | 展開下拉會重新抓取清單（非只在 mount 時） |
| 前端（守護不變式） | **選取磁碟機後，終端機再換目錄時面板仍會跟隨** |

最後一項是本設計的主要防線。既有 6 個測試（`FileExplorer.test.tsx`）**沒有任何一個覆蓋 CWD 輪詢同步**，因此現在破壞它不會有任何測試失敗。

實作時應以 mutation testing 確認該測試確實守得住：把選取磁碟機的處理改為一併設定 `ptyCwdRef`，該測試必須失敗。

## 受影響檔案

| 檔案 | 改動 |
|---|---|
| `src-tauri/Cargo.toml` | 新增 `[target.'cfg(windows)'.dependencies]` 區塊 |
| `src-tauri/src/pty/commands.rs` | 新增 `drives_from_mask` 與 `list_drives` |
| `src-tauri/src/lib.rs` | 註冊 `list_drives` |
| `src/ipc/fs.ts` | 新增 `listDrives()` |
| `src/components/FileExplorer/FileExplorer.tsx` | 工具列按鈕與下拉、選取處理 |
| `src/components/FileExplorer/FileExplorer.css` | 下拉選單樣式 |
| `src/components/FileExplorer/FileExplorer.test.tsx` | 新增測試 |
| `src/lib/i18n.ts` | 下拉選單的 aria-label / title 字串（zh-TW + en） |

## 驗證限制

`GetLogicalDrives()` 的實際行為**無法在 macOS 上驗證**——只有純函式部分測得到。完整驗證需要在 Windows 實機開啟檔案面板，確認：下拉列出的磁碟機與物件管理員一致、切換後檔案清單正確、切換後終端機 `cd` 時面板仍會跟隨、以及 macOS 上此 UI 不出現。
