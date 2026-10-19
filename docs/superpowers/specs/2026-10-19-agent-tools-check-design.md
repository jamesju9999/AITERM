# Agent 協作：檢查 AI 工具與 git 有沒有裝

日期：2026-10-19　分支：feat/agent-canvas

## 問題

新電腦沒裝 Claude Code／Codex／git 時，協作頁看起來一切正常，要等派工才失敗（0ce6078d 之後至少會立刻失敗並說明，
但使用者已經排好團隊、拖好卡片）。應該一進協作頁就告訴使用者缺什麼、怎麼裝。

## 設計

### 後端：`agents_check_tools` → `ToolsCheck`

```rust
struct ToolsCheck { platform: "macos"|"windows"|"linux", claude_command: String,
                    claude: Option<String>, codex: Option<String>, git: Option<String> }
```
每個欄位是找到的路徑，`None` = 找不到。

**要用派工時同一個環境去找**，不能看 App 自己的 PATH：
- macOS／Linux：GUI App 的 PATH 不含 Homebrew、npm、`~/.local/bin`。派工分頁是互動 shell（AITerm 的整合會
  source `.zprofile` 與 `.zshrc`），所以用 `$SHELL -ilc` 跑 `command -v`。
- Windows：派工分頁是 PowerShell（有 profile），用 `pwsh.exe`（沒有就 `powershell.exe`）跑 `Get-Command`。
  Windows 的子行程繼承 AITerm 啟動時的 PATH：剛裝好的工具要重開 AITerm 才找得到，派工也一樣，所以結果一致，
  橫幅提示要重開。
- `claude` 用設定裡的派工指令（`task_board.claude_command`）的第一個字，使用者改成完整路徑也查得對。
- 輸出用 `__AITERM_TOOL__<名稱>=<路徑>` 標記行解析，shell 啟動時印的其他東西（歡迎訊息等）不影響。
- 10 秒逾時（rc 檔卡住時不讓檢查掛著），逾時當作「無法判斷」→ 整個結果回錯誤，前端不顯示橫幅。

### 前端

- 協作頁載入時、視窗重新取得焦點時（使用者剛去終端機裝好回來）檢查一次。
- 頂列下方顯示一條橫幅，只列跟這個專案有關的缺件：
  - git：一律需要。
  - Claude Code：專案還沒有 Agent，或有任何 Agent 用 claude。
  - Codex：有 Agent 用 codex。
- 每個缺件一行：名稱、用途、該平台的安裝指令（可複製）、需要登入的說明；最後一個「重新檢查」按鈕。Windows 多一句「裝好後重開 AITerm」。
- Agent 卡片：它用的工具沒裝時，標題下顯示「⚠ 未安裝 Claude Code／Codex」。

安裝指令：
| 工具 | macOS | Linux | Windows |
|---|---|---|---|
| Claude Code | `curl -fsSL https://claude.ai/install.sh \| bash` | 同左 | `irm https://claude.ai/install.ps1 \| iex` |
| Codex | `npm install -g @openai/codex` | 同左 | 同左 |
| git | `xcode-select --install` | `sudo apt install git`（或發行版的套件管理員） | `winget install --id Git.Git -e` |

## 不做

- 自動安裝（要權限、會改使用者系統）。
- 檢查是否已登入（claude 沒有便宜可靠的非互動查法；派工失敗訊息已經會提示登入）。

## 測試

- 解析標記行（夾雜雜訊、空路徑 = 找不到、缺行 = 找不到）。
- 派工指令取第一個字（含引號路徑）。
- Unix 實際跑使用者 shell：`git` 找得到、不存在的名稱找不到。
- 前端：哪些缺件該顯示（依 Agent 的 cli）、橫幅與卡片標記的渲染。
