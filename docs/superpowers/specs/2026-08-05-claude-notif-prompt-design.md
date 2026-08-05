# 提示使用者設定 Claude Code 的 terminal bell — 設計

日期：2026-08-05
狀態：已核可，待寫實作計畫

## 問題

側邊欄終端機提示點（見 `2026-08-05-terminal-attention-badge-design.md`）靠 xterm 的 `onBell` 偵測「CLI 停下來等使用者回答」。但 Claude Code 依官方文件（terminal-config）**預設只在 Ghostty、Kitty、iTerm2 送通知**，其他終端機一律不送——AITerm 不在名單內。

後果：整個提示點功能最主要的使用情境（Claude Code 停在權限確認提示、使用者已經切到別的分頁）預設是**完全不會亮的**。而失效方式是靜默的：使用者只會覺得「這功能沒用」，不會知道少了一行設定。

解法只要在 `~/.claude/settings.json` 加一行：

```json
"preferredNotifChannel": "terminal_bell"
```

2026-08-05 已實測：設好並開新分頁後，Claude Code 的權限提示會讓背景分頁亮起橘色脈動點。

問題不在技術，在**沒有人會知道要設它**。

## 範圍

**含：** 偵測使用者在 AITerm 裡執行 `claude`、判斷設定是否缺失、跳出一次性卡片、按下同意後直接幫使用者寫進 `~/.claude/settings.json`。

**不含：**

- 偵測其他也需要類似設定的 CLI 工具。等真的遇到第二個再說。
- 修改 Claude Code 的其他設定。這張卡片只碰 `preferredNotifChannel` 一個 key。
- `Notification` hook 的替代方案。它更精確（只在 `permission_prompt` 觸發）但 hook 的 stdout 在 fullscreen TUI 下會落到哪個 tty 沒有文件保證，需要另外實測；`preferredNotifChannel` 已驗證可行且是一行設定。

## 觸發時機

**只在使用者真的在 AITerm 裡執行 `claude` 時提示。**

不在 app 啟動時檢查：那會打擾裝了 Claude Code 但從不在 AITerm 裡用它的人。而且執行當下正是時機最對的一刻——使用者接下來就會需要這個提示點。

## 偵測

### 掛在哪裡

`useTerminalBlocks` 新增 `onCommandStarted?: (cmd: string) => void`，在 `submitCommand` 與 `beginTrackedBlock` 兩處觸發。

**不掛在 `TerminalView`**：那裡有 8 處呼叫 `submitCommand`、另有一處呼叫 `beginTrackedBlock`。逐一攔截既脆弱，又會在下次有人新增呼叫點時默默失效。`useTerminalBlocks` 是這兩個函式的定義處，是唯一的漏斗。

### 比對規則

純函式，放 `src/lib/claudeCommand.ts`：

```ts
export function isClaudeCommand(cmd: string): boolean;
```

去頭尾空白 → 取第一個 token → 取 basename → 是否等於 `claude`。

| 輸入 | 結果 | 理由 |
|---|---|---|
| `claude` | ✅ | |
| `claude --resume` | ✅ | 帶參數 |
| `/usr/local/bin/claude` | ✅ | 完整路徑 |
| `  claude  ` | ✅ | 前後空白 |
| `claude-foo` | ❌ | 不同的指令 |
| `echo claude` | ❌ | 只是參數 |

環境變數前綴（`FOO=1 claude`）與 `npx claude` 刻意不支援：前者罕見，後者不是 Claude Code 的安裝方式。漏報的代價只是這次不提示，下次照樣有機會。

## 後端

新模組 `src-tauri/src/commands/claude_notif.rs`，兩個指令。

### `claude_notif_needs_prompt() -> bool`

`~/.claude/` 存在，**且** `settings.json` 的 `preferredNotifChannel` 不存在或等於 `"auto"` 時回 true。

任何其他明確值（`iterm2`、`kitty`、`ghostty`、`iterm2_with_bell`、`terminal_bell`、`notifications_disabled`）都代表使用者已經做過決定，不碰、不提示。`auto` 是預設值，在 AITerm 等於沒有通知，所以要提示。

`settings.json` 不存在但 `~/.claude/` 存在時回 true——使用者有 Claude Code，只是還沒有設定檔。

JSON 解析失敗時回 **false**：不提示。看不懂的檔案就不要碰。

### `claude_notif_enable_bell() -> Result<(), String>`

解析 `~/.claude/settings.json` → 設定 `preferredNotifChannel = "terminal_bell"` → 寫回。

**JSON 解析失敗時回傳錯誤，絕不覆寫。** 使用者的設定檔壞掉是他自己要處理的事，不是我們拿來重置的理由。

### 寫入必須是原子的

**這一節比下面的 key 順序重要得多。** key 順序是觀感問題，寫入失敗則是資料毀損。

`fs::write` 會在寫入任何一個 byte 之前先 truncate。實測過（600KB 磁碟映像檔、填滿、跑真實流程）：一個 24570 bytes 的 `settings.json` 會變成 **0 bytes**，而且錯誤訊息會如實回報——使用者看到「設定失敗」，同時他的 Claude Code 設定已經全毀。

所以寫入走「暫存檔 + rename」。同倉庫的 `src-tauri/src/config/mod.rs:141-146` 對本 app 自己的設定檔早就這樣做了，而這裡動的是別人的檔案，更沒有理由不做。

但單純換成 temp+rename 會退化兩件 `fs::write` 本來做對的事，實測數據：

| | symlink | 權限 |
|---|---|---|
| `fs::write` | 保持（寫進連結指向的真檔） | 保持 0600 |
| naive temp+rename | **連結被換成普通檔** | **放寬成 0644** |

dotfile 管理器（chezmoi / stow / yadm）常把 `settings.json` 做成連結，而設定檔是 0600 也很正常。所以三件事要一起做：**先 `canonicalize` 解 symlink → 寫暫存檔 → 從原檔複製權限 → rename**。

不加 `fsync`：`config/mod.rs` 也沒有，而 rename 已經堵住真正會傷人的「截斷」那一類。

代價是 temp+rename 需要**目錄**的寫入權限而非只要檔案的。`~/.claude` 唯讀而 `settings.json` 可寫是極罕見的組合，且失敗方式乾淨（回 Err、不動原檔）。

### 空檔案算「還沒有設定」

存在但內容為空（或只有空白）的 `settings.json` 要當成空物件處理，不是解析錯誤。

理由不只是嚴謹：**空檔案正是非原子寫入在磁碟滿時產生的東西**。若把它當成壞檔案，`needs_prompt` 會回 false，功能對這位使用者靜默失效——正是這個功能想解決的那種失效方式。

### key 順序

`Cargo.toml` 的 `serde_json` 加上 `features = ["preserve_order"]`。

沒有這個 feature 的話，`serde_json::Map` 是 `BTreeMap`，「解析 → 插入 → 序列化」會把使用者的 key **全部按字母重排**，整個檔案改頭換面。功能上無害，但在別人的設定檔上這樣做觀感很差。

選擇真正的 JSON 解析器而非定點文字插入，是因為在別人的設定檔上動手時，「key 順序改變」遠比「手刻解析寫錯把檔案弄壞」輕微。

**但這個 feature 有一個跨模組的副作用，必須一併處理。** 它把 `serde_json::Map` 從 `BTreeMap` 換成 `IndexMap`，於是 `Value` 的 `Display` 不再是正規化形式。`code_assistant/mod.rs` 與 `knowledge_base/chat.rs` 的 `tool_call_key` 正是用 `format!("{}", args)` 當 tool call 的去重 key——在 BTreeMap 下它剛好是正規化的，換之後就不是了。實測：

```
開 preserve_order：  {"path":"src","depth":3} 與 {"depth":3,"path":"src"} → 視為不同呼叫
未開：              兩者相同 → 正確去重
```

args 來自 LLM，而這個守衛存在的理由正是擋住會重複呼叫的小型／本地模型——也正是最會亂排 key 順序的那群。所以 `tool_call_key` 要自己做正規化，不能依賴 `Display`。這條在本功能之前就是隱性的脆弱假設，只是 `preserve_order` 讓它浮出來。

送往 API 的 JSON 順序改變本身無關緊要。

### 婉拒旗標

`AppConfig` 新增欄位，加上兩個指令，完全比照既有的 `appimage_integration_declined`（`src-tauri/src/commands/config.rs:31-40`）。婉拒是永久的，不再問。

## 前端

### 卡片

新元件 `src/components/ClaudeNotifPrompt.tsx`，形狀比照 `AppImageIntegrationPrompt.tsx`：沿用 `UpdateModal.css`、`role="status"`、角落卡片、accept 直接把事情做完、decline 寫進 config。

顯示條件全部成立才出現：

1. 這個 session 偵測到執行 `claude`
2. `claude_notif_needs_prompt()` 為 true
3. 沒有被婉拒過
4. onboarding 已完成
5. 沒有更高優先的角落卡片

**後端只查一次。** 使用者一次 session 裡可能跑很多次 `claude`；第一次偵測到就查詢並記住結果，之後不再重複 IPC。偵測狀態只存在記憶體，不持久化——重開 app 後若設定已寫入，`needs_prompt` 自然會是 false。

### 三張角落卡片的優先序

**更新提示 > AppImage 提示 > Claude 通知提示。**

更新最急（有安全修正的可能），AppImage 是既有行為不動它，新的排最後。`AppImageIntegrationPrompt` 已有 `hasUpdate` prop 讓位的先例，新卡片再多讓一層。

### 成功狀態必須說「要開新分頁」

Claude Code 只在啟動時讀設定，所以按下同意之後，**當前正在跑的那個 claude 不會有任何反應**。

這是使用者最可能誤判成「設了也沒用」的地方——我們自己在驗證時就踩過這一步。卡片的成功狀態必須明講「請開一個新的終端機分頁」，不能只說「已設定」。

### i18n

zh-TW + en，字串放 `src/lib/i18n.ts`。

## 測試

| 對象 | 方式 |
|---|---|
| `isClaudeCommand` | 純函式單元測試，涵蓋上表六種輸入 |
| 設定檔讀寫 | Rust + `tempfile` |
| 卡片顯示／隱藏／接受／婉拒 | Vitest，比照 `AppImageIntegrationPrompt.test.tsx` 的八個案例 |

Rust 測試必須涵蓋的六種情況——**這是全功能最需要測試的部分，因為它動的是別人的檔案**：

1. `settings.json` 不存在 → 建立，只有這一個 key
2. 既有多個 key → 全部保留，順序不變，只多出新的 key
3. 已經是 `terminal_bell` → `needs_prompt` 為 false
4. 值是 `auto` → `needs_prompt` 為 true，寫入後覆蓋成 `terminal_bell`
5. 值是 `iterm2` → `needs_prompt` 為 false，不動
6. JSON 壞掉 → `needs_prompt` 為 false；`enable_bell` 回錯誤且**檔案內容未改變**

## 手動驗證

自動測試涵蓋比對規則、檔案讀寫、卡片渲染。**偵測的接線（`useTerminalBlocks` → `TerminalApp` → 卡片）只有手動驗證能保護**，與提示點功能的情況相同。

必做步驟：

1. 先把 `~/.claude/settings.json` 的 `preferredNotifChannel` 移除（或改成 `"auto"`）
2. 在 AITerm 執行 `claude` → 卡片應出現
3. 按下同意 → 確認 `~/.claude/settings.json` 多了那一行，**且其他 key 一字未改、順序不變**
4. **開一個新的終端機分頁**跑 `claude`，讓它停在權限確認，切到別的分頁 → 應亮橘色脈動點
5. 重開 app，再執行一次 `claude` → 卡片不應再出現（設定已存在）
6. 另外測婉拒路徑：還原設定、婉拒一次、重開 app、再跑 `claude` → 卡片不應出現

## 已知限制

**只涵蓋使用者直接輸入 `claude` 的情況。** 包在腳本裡、透過 alias、或帶環境變數前綴啟動的都偵測不到。漏報的代價只是這次不提示。

**設定寫入後不影響已在執行的 claude session。** 見〈成功狀態必須說「要開新分頁」〉。

**只處理使用者層級的 `~/.claude/settings.json`。** 專案層級（`.claude/settings.json`）與企業管理設定不在範圍內。

**與 Claude Code 本身的並行修改沒有互斥保護。** 我們的 read-modify-write 不是原子的：使用者按下按鈕的那一刻若 Claude Code 正在寫 `settings.json`（例如 onboarding 寫 `hasCompletedOnboarding`／`theme`，而那正是 `claude` 啟動時、也正是卡片跳出來的時候），兩邊會互相蓋掉。

視窗是毫秒級而使用者點擊要好幾秒，實務風險低。真要解需要檔案鎖，而 Claude Code 自己沒有加，我們單方面加也擋不住對方——寫成已知限制比做半套誠實。

**CRLF 與數字格式會被正規化。** 走 `Value` round-trip 之後，CRLF 行尾會全部變成 LF，`1e10` 這種寫法會變成 `10000000000.0`。前者是純觀感，後者機率極低；都**不要**用 `arbitrary_precision` 去修——那會全域改變 `Value` 的形狀並破壞其他模組的 `as_f64` 比對。

## 跨平台

`~/.claude/settings.json` 的路徑在 macOS / Windows / Linux 上都由 home 目錄推導，用既有的 `dirs` crate（`Cargo.toml` 已有依賴）取得，沒有平台專屬邏輯。指令比對是純字串處理，但 basename 切分要同時處理 `/` 與 `\` 兩種分隔符號。
