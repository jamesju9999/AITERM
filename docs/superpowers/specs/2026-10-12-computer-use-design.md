# GUI 控制與截圖（Computer Use）設計

日期：2026-10-12

## 目標

讓 AI 能看見並操作 macOS 上的其他應用程式：列出 App、讀無障礙元件樹、截圖、點擊、打字、按鍵、捲動。對外是一組 MCP 工具（外部 Claude Code／Codex 可用），AITerm 內建 Agent 經由自己的 MCP client 使用同一組工具。

## 範圍

做：macOS 後端、七個工具、授權與危險操作確認、稽核紀錄、設定頁（總開關、權限狀態、已授權 App、稽核紀錄）。

不做：Windows／Linux 後端（trait 已保留，回傳「尚未支援」）、內建 Agent 直接看到截圖影像（見「已知限制」）、錄影、拖曳、多螢幕選擇。

## 架構

`src-tauri/src/computer/`：

| 檔案 | 職責 |
|------|------|
| `mod.rs` | `ComputerService`：所有工具的唯一入口，串起 policy → approver → backend → audit |
| `backend.rs` | `Backend` trait 與資料型別（`AppInfo`、`ElementNode`、`Shot`）；非 macOS 的 `UnsupportedBackend` |
| `macos.rs` | macOS 實作：AX（ApplicationServices FFI）、CGEvent（core-graphics）、`screencapture`／`sips`／`osascript` |
| `policy.rs` | 純邏輯：危險關鍵字判斷、不可操作對象（AITerm 自己）、按鍵名稱解析 |
| `approval.rs` | `Approver` trait；`TauriApprover` 以事件＋oneshot 向前端要使用者決定 |
| `audit.rs` | 稽核紀錄（JSON Lines）寫入與讀取最後 N 筆 |

兩條入口（MCP server、之後任何入口）只能呼叫 `ComputerService`，沒有繞過 policy 的路徑。

## 工具

| 工具 | 參數 | 回傳 |
|------|------|------|
| `computer_list_apps` | — | 前景可見 App：名稱、bundle id、pid、是否最前 |
| `computer_snapshot` | `app`、`max_depth?` | 元件樹文字：`id role "title" (x,y,w,h)`；id 為子節點索引路徑 |
| `computer_screenshot` | `app?` | PNG 影像（縮到邏輯像素寬）＋文字（尺寸、原點座標）；有 `app` 時裁到其最前視窗 |
| `computer_click` | `app`、`element?` 或 `x,y`、`button?`、`double?` | 結果文字；優先 AXPress，失敗退回座標點擊 |
| `computer_type` | `app`、`text`、`element?` | 結果文字；有 `element` 時先聚焦 |
| `computer_key` | `app`、`key`（如 `cmd+s`、`return`） | 結果文字 |
| `computer_scroll` | `app`、`dx`、`dy`、`x?`、`y?` | 結果文字 |

`app` 以名稱或 bundle id 指定，必須是 `computer_list_apps` 看得到的 App。

## 授權與安全

1. 設定 `mcp_tool_server.computer_enabled`（預設 false）是總開關；關閉時所有 `computer_*` 工具回傳說明如何開啟。
2. **每個 App 首次授權**：對 App 做第一個會改變狀態或讀取內容的動作（除 `list_apps` 外皆是）前，彈出授權（允許一次／永遠允許／拒絕）。「永遠允許」存進 `computer_allowed_apps`（bundle id）。
3. **破壞性操作逐次確認**：`click` 的目標元件（以 element id，或用 AX 查座標處的元件）標題／描述符合關鍵字（刪除、移除、清空、送出、發送、付款、購買、結帳、Delete、Remove、Erase、Send、Pay、Purchase、Submit、Empty Trash…）時，即使已永遠允許也每次確認；`key` 的 `cmd+delete` 同理。`type` 的文字內容不掃描（在文字欄打「delete」不是破壞性操作），也不寫進稽核紀錄，只記字元數。「永遠允許」在破壞性確認中只當作這一次。
4. **禁止操作 AITerm 自己**（pid 比對），避免 Agent 點掉自己的確認框。
5. 授權請求來自 MCP 時，先把 AITerm 視窗叫到前景；60 秒沒回應視為拒絕。
6. 每次呼叫（含被拒絕）寫稽核紀錄：時間、工具、App、摘要、結果。
7. 缺權限（輔助使用、螢幕錄製）時回傳指出缺哪一項與去哪開的錯誤；不靜默失敗。

## 設定與 UI

- `McpToolServerConfig.computer_enabled: bool`、`AppConfig.computer_allowed_apps: Vec<String>`（皆 `#[serde(default)]`）。
- MCP Tool Server 設定頁新增「電腦操作」區塊：總開關、輔助使用／螢幕錄製狀態與「前往系統設定」、已永遠允許的 App（可移除）、最近稽核紀錄。
- 前端監聽 `computer://approval-request`，顯示授權對話框，經 `computer_resolve_approval` 回覆。

## 已知限制

- 內建 Agent 的 MCP client 只保留文字內容，`computer_screenshot` 的影像在內建 Agent 看不到，只看得到尺寸文字；內建 Agent 以 `computer_snapshot` 為主。外部 MCP 客戶端（Claude Code 等）可收到影像。把影像送進 provider 的 tool-result 需要各 provider 的支援，另開一輪。
- 僅 macOS。
- 元件樹不展開選單彈出層（`AXMenu`），只保留選單列標題，避免行數預算被上百個看不見的選項吃光。
- 元件 id 是索引路徑，畫面變動後可能指到別的元件；點擊時比對 snapshot 當時記下的 role／title，不符回傳「畫面已變，請重新 snapshot」。

## 測試

- 單元：關鍵字判斷（含大小寫、中英）、自身 pid 拒絕、按鍵名稱解析、授權狀態機（FakeApprover＋FakeBackend 走完 首次授權／永遠允許／拒絕／逾時／危險逐次）、稽核紀錄讀寫、元件樹格式化、id 路徑解析與 stale 偵測、非 macOS 後端。
- 真機（macOS）：開 TextEdit，走 list → snapshot → type → key → screenshot，確認危險按鈕被攔下。
