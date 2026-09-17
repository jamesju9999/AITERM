# Windows 提權（UAC）PTY session：權限不足時就地切換成系統管理員身分

日期：2026-09-17

## 起因

AITerm 在 Windows 上以一般身分執行。當使用者在終端機裡打了一條需要系統管
理員權限的指令（例如寫入 `Program Files`、改機碼），指令會直接失敗，使用
者得自己意識到問題、手動用管理者身分重開一個 shell 去跑。AITerm 目前完全
不介入這個流程，AI（`ai_query`/`ai_chat`）看到的也只是失敗的錯誤訊息，沒
有辦法建議或代為重跑。

`ai_query`（`src-tauri/src/commands/ai.rs:344-354`）與 `ai_chat` 都是靠
`context::snapshot(&pty_manager, &session_id)`（`src-tauri/src/ai/context.rs:14-38`，
內部呼叫 `pty_manager.get_recent_output(session_id, 4096)`）取得該分頁最近
的終端機輸出。`PtyManager`（`src-tauri/crates/aiterm-core/src/pty/manager.rs:12-14`）
是純粹的 `HashMap<String, Arc<PtySession>>`，不分 session 種類。這代表：只
要提權指令的輸出最終進了同一個 `session_id` 底下的 ring buffer，AI 就會自
動看到，完全不用改 AI 那一層的程式碼——這是本次設計刻意依賴的既有機制。

## 目標

- 使用者打的指令因權限不足失敗時，AITerm 主動偵測並詢問是否要用系統管理
  員身分重跑。
- 同意後只跳一次 Windows UAC 提示；提權後的 shell 持續存在，同分頁後續的
  提權指令不用再跳 UAC。
- 提權指令的輸出跟一般指令的輸出在**同一個終端機畫面**交錯顯示，使用者體
  驗上像同一個連續的 session。
- AI 不需要額外被告知「去查另一個 session」——提權輸出直接併入原本那個
  session 的 context。

非目標：

- 不支援 macOS/Linux。兩者的 `sudo` 模型已經在 shell 自己的層級處理，不需
  要疊這套機制，比照 DB2 sidecar 的 Windows-only 前例
  （`docs`／`CLAUDE.md` 既有慣例，用 `tauri.windows.conf.json` 覆寫）。
- 不支援 cmd.exe / PowerShell 以外的 shell（WSL、git-bash 本身已有自己的
  權限模型）。
- 不做「手動提權按鈕」——本版只做自動偵測後詢問；手動切換可以是之後的獨
  立功能。

## 架構：獨立的提權宿主 sidecar

新增 workspace crate `src-tauri/crates/aiterm-elevated-host`（加進
`src-tauri/Cargo.toml` 的 `[workspace] members`），編譯成獨立執行檔
`aiterm-elevated-host.exe`。刻意不依賴 `aiterm-core`／Tauri／webview，只依
賴 `portable-pty`（跟主程式一致，重用同一套 ConPTY 建立邏輯的經驗，但不共
享行程），把提權行程的攻擊面降到最低。

**這個 crate 必須在 macOS/Linux 上也能编譯成功**（即使功能上不會被呼叫），
因為 `cargo test --workspace` 是本專案的標準檢查流程
（見專案 `CLAUDE.md`）。做法：所有 ConPTY/`ShellExecuteExW` 相關程式碼包
在 `#[cfg(windows)]`，非 Windows 平台的 `main()` 直接印錯誤訊息並以非零
狀態碼結束。

### 啟動流程

1. 主行程（一般身分，`src-tauri/crates/aiterm-core/src/pty/manager.rs`
   所在的行程）產生一個唯一具名管線名稱（`\\.\pipe\aiterm-elevate-{session_id}-{random}`），
   自己先建好 pipe server 等待連線。
2. 主行程呼叫 `ShellExecuteExW`，`lpVerb = "runas"`，目標是
   `aiterm-elevated-host.exe <pipe-name> <shell-variant>`。這一步跳出 UAC
   對話框。
3. 提權後的 sidecar 啟動，以 client 身分連進步驟 1 開好的管線——不依賴繼
   承 handle（UAC broker 本來就不轉送 stdio 的 handle 繼承，靠管線名稱字
   串繞開這個已知限制）。
4. sidecar 呼叫 `CreatePseudoConsole` 開一個新的 ConPTY，掛上
   `cmd.exe` 或 `powershell.exe`（依主行程傳入的 shell-variant，對齊該分
   頁原本用的 shell）。
5. sidecar 讀 ConPTY 輸出 handle，包成 length-prefixed frame 寫進管線；主
   行程讀到後**直接寫進原本那個 session 的 output ring buffer**、發在同
   一個 `pty://data/{sessionId}` 事件上（沿用
   `src-tauri/crates/aiterm-core/src/pty/session.rs` 既有的輸出流機
   制）。前端 `TerminalView.tsx` 不需要改動即可顯示提權輸出——它以為這
   仍是同一個 session 的資料。
6. 使用者在提權模式下打字時，主行程改把 keystroke 轉送進管線給 sidecar
   寫入 ConPTY input，而不是原本一般身分子行程的 stdin。
7. sidecar 的提權 shell 沒被使用者 `exit`、管線也沒斷之前持續存在；同分
   頁後續被判定需要提權的指令，直接沿用步驟 6 的路由，不再跳 UAC。

### session 狀態

`PtySession` 新增一個欄位 `elevated: Mutex<Option<ElevatedChannel>>`。
`ElevatedChannel` 持有：具名管線的寫入端、sidecar 行程控制 handle、目前
是否已連線的狀態。有值時，`pty_write`（`src-tauri/src/pty/commands.rs:49`）
這條指令路徑改寫進 `ElevatedChannel`；沒有值（或 sidecar 已離線）時維持原
本寫進一般子行程的行為。

管線的 frame 協定：4 bytes little-endian 長度前綴 + 1 byte 訊息種類
（`Data` / `Resize` / `Exit`）+ payload。`Resize` 由主行程在分頁改變大小
時主動送，讓提權 ConPTY 跟一般 PTY 保持同樣的 cols/rows。

## 觸發流程：自動偵測權限不足

- 主行程在每次 PTY 輸出穩定下來後（沿用現有的輸出 debounce 機制），用
  shell-variant 專屬的正則掃最近輸出：
  - cmd.exe：`Access is denied.`、`You do not have sufficient privilege`
  - PowerShell：`UnauthorizedAccessException`、
    `Access to the path .* is denied`、`requires elevation`
- 規則只在「剛執行完一條指令、輸出區塊結尾附近」判定，不是整個 scrollback
  全文比對，避免使用者自己 `echo` 出這些字串時誤判。
- 命中後，後端發一個新事件 `pty://elevation-suggested/{sessionId}`，payload
  帶剛剛失敗的那條指令文字。前端在終端機內顯示一個 inline banner：
  「此指令似乎需要系統管理員權限，要用系統管理員身分重新執行嗎？」
  [是] [否]。
- 使用者按「是」→ 呼叫新 IPC command `pty_elevate(session_id)`：
  - 若該 session 尚未有 `elevated` channel，跑上一節的完整啟動流程。
  - 若已經有（同分頁先前已提權過），直接沿用既有 channel。
  - 連線就緒後，自動把剛剛失敗的那條指令文字重送一次到提權 shell 執行，
    使用者不用重打。
- 使用者按「否」→ 什麼都不做，banner 關閉。

## UI

分頁在提權模式期間（`elevated` 有值）顯示一個小徽章（沿用
`docs/superpowers/specs/2026-09-11-shell-identity-badge-design.md` 已經有
的分頁徽章樣式與掛載位置，同一排並排顯示），文字為「系統管理員」並用橘色
系跟一般的 shell 身分徽章區分。離開提權模式（`exit` 或連線中斷）徽章消
失。

inline banner 與提權/離開提權的系統訊息文案放進 `src/lib/i18n.ts`，中英各
一份。

## 錯誤處理

- 使用者在 UAC 對話框按「取消」→ `ShellExecuteExW` 回
  `ERROR_CANCELLED`，主行程顯示一則「已取消系統管理員權限」提示，`elevated`
  維持 `None`，一般 session 不受影響。
- sidecar 中途當掉或管線斷線 → 主行程偵測到管線讀取錯誤/EOF，清空
  `elevated`、路由切回一般子行程，並在終端機印一行系統訊息「系統管理員
  連線已中斷」。
- 使用者在提權 shell 裡打 `exit` → sidecar 偵測到子行程結束，主動關閉管
  線；主行程收到 EOF 走跟上一條一樣的清理路徑，但印的是「已離開系統管理
  員模式」，不算錯誤。
- 分頁被關閉時，若 `elevated` 有值，連帶終止 sidecar 行程（避免殘留提權
  行程掛著，呼應 memory 裡「派工分頁與 claude 行程都不會自動清」踩過的同
  類坑，這次要在分頁關閉路徑裡明確處理）。

## 平台範圍

`pty_elevate`、`pty://elevation-suggested/*` 等新增的 command/event 只在
Windows 上註冊功能；macOS/Linux 上 `pty_elevate` 回傳
`AiError`-風格的 `not_supported` 錯誤（或前端直接不顯示 banner／徽章的觸
發路徑，比照 DB2 的 Windows-only 模式）。偵測正則本身也只在
`shell_variant` 為 `Cmd`/`PowerShell` 時執行。

## 建置/打包

- 新增 `scripts/setup-elevated-host-win.ps1`：`cargo build --release -p
  aiterm-elevated-host`，把產出的 exe 複製進 `src-tauri/binaries/`，用
  Tauri `externalBin` 要求的 target-triple 命名（比照現有
  `binaries/uv` 的做法）。
- `src-tauri/tauri.windows.conf.json` 的 `externalBin` 加入這個新二進位
  路徑。**只加在 windows 這份 conf**，不影響 mac/linux 的 `externalBin`
  清單。
- 因為 `tauri-build` 的 `build.rs` 會在編譯期驗證每個 `externalBin` 項目
  是否存在於磁碟（`CLAUDE.md` 已記錄的已知地雷——DB2/uv 都踩過），這裡要
  在 `CLAUDE.md` 的建置指令表格追加一行，提醒 Windows 開發者：沒先跑
  `setup-elevated-host-win.ps1`，連 `cargo check`/`cargo test` 都會在编譯
  期失敗。
- `src-tauri/binaries/` 本身已是 gitignored，這個新二進位不例外。

## 測試

Rust（`aiterm-elevated-host` crate 與 `aiterm-core`）：

- frame 編解碼（長度前綴 + 訊息種類）的往返測試。
- 偵測正則對正例／反例 fixture：正例是真實的 cmd.exe/PowerShell 權限不足
  輸出樣本；反例必須包含「字面上像但不該觸發」的輸出（例如使用者自己
  `echo "Access is denied."`、或訊息出現在 scrollback 中段而非剛執行完的
  結尾），確保 fixture 真的會區分正確與錯誤的判定結果。
- sidecar 的 ConPTY + 管線轉送邏輯：加一個測試專用旗標跳過
  `ShellExecuteExW`，直接以目前身分啟動 sidecar 子行程，驗證雙向位元組轉
  送與 `Resize` frame 正常運作。真正的 `runas`/UAC 彈窗是互動式系統對話
  框，CI 測不到，這件事在測試檔案跟這份 spec 都要明講，留給實機手動驗
  證。
- `PtySession.elevated` 狀態機：未連線／已連線／連線中斷三種狀態下
  `pty_write` 路由到正確目的地。

前端：

- inline banner 的顯示／「是」／「否」互動，IPC 呼叫用 mock。
- 提權徽章依 `elevated` 事件狀態正確顯示/消失。
- 分頁關閉時，若當下是提權狀態，驗證有呼叫終止提權 channel 的清理邏輯。

## 風險與取捨

- **無法在這台開發機（macOS）上實機驗證 UAC/ConPTY 提權流程**。所有
  Windows 專屬邏輯只能先靠邏輯測試 + 手動在 Windows 機器上驗收，這點會
  在實作計畫裡列成明確的驗收步驟，不能只靠 CI 綠燈就宣告完成。
- **偵測正則會有誤判**：不管是漏抓（真的需要提權但沒偵測到）還是誤觸發
  （非權限問題也跳 banner），代價都只是多一次可以按「否」的詢問，不會
  自動執行任何提權動作，風險可控。
- **提權指令重送機制**（步驟：連線就緒後自動重跑失敗指令）假設該指令是
  幂等或至少重跑安全；如果原指令有副作用（例如已經部分寫入檔案），重跑
  可能造成非預期結果。這點先接受，之後如果有真實案例回報再收斂成「只對
  已知安全的指令類型自動重送，其餘只切模式不重送」。
- **一個分頁同時只允許一個提權 channel**：不支援巢狀或多重提權 shell。若
  使用者需要，得先 `exit` 目前的提權 shell 才能重新提權（例如換一種
  shell-variant）。目前沒有已知需求要同時開兩個，先不做。
