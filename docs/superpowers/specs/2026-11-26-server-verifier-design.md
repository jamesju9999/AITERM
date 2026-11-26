# Server 驗收者（共用看板的 Verifier）

日期：2026-11-26　狀態：設計已與使用者確認（2026-11-26），待寫實作計畫

承接：`2026-10-17-agent-verifier-design.md`（桌面版驗收者）、`2026-11-25-board-server-m2-m3-design.md`（server 無頭編排，S6 結論把驗收者列為「不做」）。

## 1. 為什麼（以及先前的判斷哪裡錯了）

M2／M3 的 S6 結論說驗收者「要實際操作應用程式畫面，server 無頭環境沒有」，所以不做。重新檢視後這個理由不成立：

- 桌面版驗收者**不接管螢幕**（桌面版 spec §5、§11 明寫不做電腦操作）。它在隔離的 worktree 裡啟動專案，用**無頭 Playwright 瀏覽器**點擊、截圖，加上命令列驗證。無頭 server 做得到。
- 使用者的 server 大多裝在桌面電腦上、用自己的帳號，Chrome 與 Node 通常已經有。

做不到的只剩兩件事，都沿用桌面版的 `manual` 標記處理：原生桌面 App 畫面、中途詢問人類（`claude -p` 非互動，沒有提問管道）。

沒有驗收，server 自己跑完的成果只經過「單一模組的 diff 審查」，模組之間有沒有真的串得起來沒人驗。

## 2. 已拍板的決定

1. **時機**：驗收在**管理員核准之前**，於**拋棄式整合 worktree** 進行。通過才出現「合併待核准」。管理員核准仍是唯一的合併關卡，「永不自動合併」不變。
2. **退回**：自動退回給對應工程師，最多 2 輪；找不到模組或超過輪數就停下來等管理員（可放行或重新驗收）。
3. **命令權限**：開放 Bash，加**可維護的禁止清單**（全域預設＋專案覆寫），不做白名單。使用者的團隊都是信任的人，風險主要是 Claude 誤操作，不是惡意。
4. **啟用**：每個專案由管理員手動開啟「由 server 執行驗收」，且畫布上要有驗收者角色。沒開就照現在的流程，舊專案不受影響。
5. **報告**：第一版就含截圖縮圖。
6. **範圍**：一份 spec、三個里程碑（後端核心／報告與畫面／真實 Claude 實機驗收）。

## 3. 角色與卡片

- Agent 種類 `verifier` 已存在（桌面版）；server 的 `board_agents.kind` 本來就收它，每個專案最多一位。
- 新卡片種類 `verify`（`card_kind = "verify"`）。不沿用 `review` 加欄位，避免弄亂審查鏈。
- 修正卡新種類 `vfix`（驗收退回的修正卡）。它在模組鏈裡像 `fix` 一樣接在鏈尾，但**不計入審查的退回輪數**。
- 驗收卡 `module_id` 為空（它屬於整個專案，不屬於單一模組），`review_of` 為空，`agent_id` 為驗收者。

## 4. 流程與狀態

```
全部模組 Passed ──(驗收者存在且專案開了驗收)──► 建 verify 卡（queued）
                                                  │ server 領取
                                                  ▼
                            建整合 worktree（專案目前版本 + 依序合併各工程師分支）
                                                  │
                        合併衝突 ─► 驗收卡失敗（原因：合併衝突），停下等管理員
                                                  ▼
                                   Claude（驗收模式）跑，寫 verify.json
                 ┌────────────────────────┼──────────────────────────┐
               pass                      fail（有 module_id）       fail（找不到模組）
                 ▼                          ▼                          ▼
        建「合併待核准」卡           每個模組開 vfix 卡           verify 卡 verdict=attention
        附驗收報告                   → 審查 → 再驗收               停下等管理員（放行／重新驗收）
```

### 4.1 什麼時候建驗收卡

`advance` 現在在「`merge_status == Ready` 且沒有待核准的合併卡」時建合併卡。改成：

- 專案**啟用驗收**（開關開、有驗收者）→ 改建 `verify` 卡，**不建**合併卡。
- 沒有啟用 → 照舊建合併卡。
- 同一組模組版本不重複建驗收卡：用「各模組通過版本（`sha`）排序後的雜湊」當簽章存在 verify 卡上，簽章相同且已有未被推翻的 verify 卡就不再建。工程師分支 HEAD 變了（修正卡做完）簽章就變，會建新的。

### 4.2 通過

- `verify` 卡 `verdict = "pass"`、`outcome = "success"`。下一個 tick `advance` 看到「最新簽章的 verify 卡通過」→ 建合併卡，內文附驗收結論摘要。
- `manual` 項目不擋通過，但驗收卡 `verdict` 旁記 `manual_count`，合併卡與畫面顯示「N 項需你確認」。

### 4.3 不通過

- 每個有 `module_id` 且對得上模組的失敗項，依模組分組，各開一張 `vfix` 卡給原工程師：
  - 內文：驗收者寫的重現步驟、預期與實際、截圖檔名，加模組原始需求。
  - `module_id` = 模組；`review_of` = 該模組鏈目前的尾端（通過的那張審查卡；沒有審查者時是最後一張成功的卡）。
- 修好後照原路走：審查 → 模組回到 Passed → 簽章變了 → 建新的 verify 卡。
- **輪數**：從最近一次 `pass` 之後，連續 `fail`／`attention` 的 verify 卡張數 ≥ `MAX_FIX_ROUNDS`（2）→ 不再開 vfix 卡，verify 卡 `verdict = "attention"`，停下等管理員。
- 失敗項**全部**找不到模組 → `verdict = "attention"`；有些找不到、有些找得到 → 找得到的照開 vfix 卡，找不到的寫在報告裡。
- **管理員放行**：把該 verify 卡標為通過（`verdict = "pass"`、記 `overridden_by`），下一個 tick 建合併卡。
- **管理員重新驗收**：對 `fail`／`attention` 的 verify 卡建一張新的（簽章加一個重試序號），不必等工程師改東西。

### 4.4 模組狀態（最容易出錯，先寫測試再改）

`orchestration::module_state` 走鏈時（`next_of`）會把 `fix` 計入 `fixes`，`fixes >= MAX_FIX_ROUNDS` 且審查 fail 就是 `Exhausted`。要加的規則：

- 鏈裡出現 `vfix`：走到它時**不增加 `fixes`**。
- 模組原本 `Passed`，驗收退回開了 `vfix` 後，鏈尾變成 `vfix`（未完成）→ `Waiting`，**不能還是 `Passed`**（否則 `merge_status` 在修好前又判 Ready，會建出新的 verify 卡）。
- `vfix` 做完 → `NeedsReview`（鏈尾不是 `review` 且有審查者）→ 審查 → `Passed`。
- 驗收卡不在模組鏈裡（`module_id` 為空），所以 `modules()` 與 `module_state` 不需要認識 `verify`。

這幾個情境在計畫裡要先寫成 Rust 測試。

### 4.5 合併卡的閘門

`advance` 建合併卡的條件（`pending_merge` 之外）加上：有啟用驗收時，必須存在「簽章等於目前簽章」且 `verdict = "pass"` 的 verify 卡。

## 5. 驗收怎麼跑

### 5.1 整合 worktree

- 位置：`<資料夾>/worktrees/verify-<驗收卡 id 前 12 碼>`，detached，從專案工作資料夾目前的版本開。
- 依序把各 Coder 的分支（`agent/<id 前 12 碼>`，只含有通過模組的）`git merge --no-ff` 進去。任何一個衝突 → `merge --abort`、驗收卡失敗，原因寫「合併衝突：哪個分支」。
- 驗收卡結束（不論成敗）→ `git worktree remove --force`＋`prune`。驗收產生的建置檔、`node_modules` 都留在這個 worktree，不會 commit，也不會碰專案資料夾。

### 5.2 驗收模式

`runner::Mode` 新增 `Verify { deny: Vec<String>, ports: [u16; 3], mcp_config: Option<PathBuf>, evidence_dir: PathBuf }`：

- `--allowedTools`：`Read,Grep,Glob,Bash`＋（有 Playwright 時）`mcp__playwright`。**不給 Edit／Write／MultiEdit**——驗收者不改程式；`verify.json` 與截圖由 Playwright 與驗收卡資料夾寫入（見 5.4）。
- `--disallowedTools`：禁止清單（見 §6）＋`WebFetch,WebSearch,NotebookEdit,Edit,Write,MultiEdit`。
- `--permission-mode dontAsk`、逾時（沿用 `RunnerSettings.timeout_secs`，驗收另有放寬值 `verify_timeout_secs`，預設 1800）、`--max-turns`（驗收另有 `verify_max_turns`，預設 120）。
- 提示詞與桌面版共用內容（專案類型表、驗收條件來源、`verify.json` 格式），server 版差異：沒有 `ask_human`（需要問人的條件一律標 `manual` 並寫原因）、埠與資料夾由 server 指定。
- 對話記錄：驗收用 §「中途接續」同一套機制（`--session-id`／`--resume`），額度用完或重啟後接著做。

### 5.3 瀏覽器

沿用桌面版 §6：

- `--mcp-config <檔>`＋`--strict-mcp-config`，內容只有 `playwright`：`npx -y @playwright/mcp@0.0.83 --headless --isolated --output-dir <證據資料夾>`（版本釘死）。
- 系統有 Chrome 用預設通道；沒有就加 `--browser chromium`，提示要求遇到 "not installed" 先跑 `npx -y @playwright/mcp@0.0.83 install-browser chrome-for-testing` 再重試（桌面版已實測）。偵測沿用桌面版的路徑清單。
- 沒有 Node／npx：不放 `mcp__playwright`，提示要求需要瀏覽器的條件標 `manual` 並寫原因，不能當通過。

### 5.4 證據與 verify.json

- 證據資料夾：`<資料夾>/verify/<驗收卡 id>/evidence/`，`--output-dir` 指向它；提示要求截圖一律存成該資料夾底下的絕對路徑（桌面版實測：`--output-dir` 底下的絕對路徑可寫，其他位置被擋）。
- `verify.json` 由驗收者**用 Bash 寫**到 `<資料夾>/verify/<驗收卡 id>/verify.json`（沒有 Write 工具）。格式同桌面版 §7：`verdict`、`project_type`、`items[{criterion, status(pass|fail|manual), module_id?, evidence[], steps?, expected?, actual?}]`、`notes`。`verdict` 與 items 不一致時以 items 為準（任何一項 `fail` 就是 `fail`）。`manual` 一定要寫原因。
- 檔案沒寫或解析失敗 → 驗收卡 `outcome = failed`（原因寫明），不算通過；可「重新驗收」。
- server 讀到後把解析結果存進資料庫（見 §7），證據檔留在磁碟。

### 5.5 埠與收尾

- 每次驗收 server 先找 3 個空閒埠，寫進提示，要求伺服器只用這 3 個。
- 驗收結束（不論成敗）：找出仍在監聽這 3 個埠的行程並結束（macOS／Linux `lsof -ti tcp:<port>`；Windows `netstat -ano` 再 `taskkill`）。**只動這 3 個埠**。Playwright MCP 是 claude 的子行程，claude 結束時跟著結束。
- server 重啟時：沿用「結束孤兒 claude」，再依記錄的驗收埠補一次清理。
- 環境變數沿用 `minimal_env`（不帶 server 的祕密）。

## 6. 命令權限與禁止清單

### 6.1 兩層清單

- **全域預設**：整台 server 一份（`server.db` 設定），管理員編輯。初版內容：

| 類別 | 規則（`--disallowedTools` 的 `Bash(...)`） |
|---|---|
| 提權 | `sudo:*`、`su:*` |
| 對外發佈 | `git push:*`、`npm publish:*`、`cargo publish:*`、`gh pr create:*`、`gh release create:*`、`docker push:*` |
| 遠端連線 | `ssh:*`、`scp:*`、`sftp:*` |
| 下載後直接執行 | `curl:* \| sh`、`curl:* \| bash`、`wget:* \| sh`、`wget:* \| bash` |
| 破壞性刪除 | `rm -rf /:*`、`rm -rf ~:*`、`rm -rf $HOME:*`、`rm -rf ..:*` |
| 改系統設定 | `launchctl:*`、`systemctl:*`、`defaults write:*` |
| 改全域環境 | `npm install -g:*`、`npm i -g:*`、`pip install --user:*` |

- **專案覆寫**：只能「移除某條預設」或「新增專案專屬的規則」，不影響別的專案。
- 每條規則有一行說明、可停用；可一鍵還原預設。
- 只有管理員能改；改了下一次驗收立即生效（每次驗收前才讀）；每次異動寫稽核紀錄（誰、何時、哪一條、動作）。
- 規則字串要通過格式檢查（必須是 `Bash(...)` 的形式、不含換行、長度上限），不合法的拒絕存。

### 6.2 被擋的命令要看得到

驗收卡結束後，server 從 Claude Code 的輸出找出被拒絕的 Bash 呼叫（`permission_denials`），存進報告；畫面列出「本次被禁止清單擋下的命令」，管理員據此決定要不要放寬。

### 6.3 誠實的限制

這不是沙箱：只擋最明顯的誤操作。Claude 若把命令寫進腳本再執行（`bash x.sh`），字串比對攔不住。Bash 理論上仍能碰到 worktree 外的檔案。使用者的團隊是單一信任範圍，這個防護程度被接受；需要真隔離時要另做容器化（不在這一版）。

### 6.4 其他限制

- 每個專案由管理員手動開啟「由 server 執行驗收」，操作寫稽核紀錄。
- 驗收在拋棄式 worktree 內以該目錄為工作目錄執行；有逾時與回合上限。

## 7. 資料與 API

### 7.1 資料

- `board_cards` 不加欄位（`verify`、`vfix` 只是新的 `card_kind`）。
- 新表 `board_verify`（一張驗收卡一列）：`card_id`、`signature`（簽章）、`ports`（JSON）、`report`（解析後的 verify.json，JSON）、`denied`（被擋命令，JSON）、`manual_count`、`overridden_by`、`created_at`。
- 新表 `board_verify_deny`（專案覆寫）：`project_id`、`rule`、`kind`（`add`｜`remove`）、`enabled`。全域預設放 `server.db` 的設定。
- 專案設定新增 `verify_enabled`（沿用 `orchestrator::enabled` 同一種儲存方式，鍵名 `verify_enabled:<pid>`）。

### 7.2 API（需登入）

| 端點 | 權限 | 說明 |
|---|---|---|
| `GET /v1/board/projects/{pid}/verify` | 成員 | 驗收設定（開關、輪數狀態、目前有效的禁止清單） |
| `PUT /v1/board/projects/{pid}/verify` | 管理員 | 開／關驗收 |
| `GET /v1/board/cards/{id}/verify` | 成員 | 驗收報告（解析後 JSON＋證據檔名＋被擋命令） |
| `GET /v1/board/cards/{id}/verify/files/{name}` | 成員 | 證據檔（截圖）；檔名只能是證據資料夾底下的單一檔案，擋 `..` 與子目錄 |
| `POST /v1/board/cards/{id}/verify/override` | 管理員 | 放行（視為通過） |
| `POST /v1/board/cards/{id}/verify/retry` | 管理員 | 重新驗收 |
| `GET/PUT /v1/verify/deny` | 管理員 | 全域禁止清單 |
| `GET/PUT /v1/board/projects/{pid}/verify/deny` | 管理員 | 專案覆寫 |

事件通知：驗收卡狀態變動走現有的 `changed` 事件，不新增事件種類。

## 8. 畫面（共用畫布）

- 驗收者卡片狀態：驗收中／✓ 通過（N 項需你確認）／✗ 退回 N 項／⚠ 需要你決定。
- 點驗收卡開**驗收報告**：每個條件的狀態、重現步驟、預期與實際、截圖縮圖（點開放大）、被擋命令；管理員有「放行」「重新驗收」。
- 合併卡附驗收結論與報告入口。
- 畫布頂列：驗收中 →「驗收中」；通過 → 可核准合併；退回 →「驗收退回 N 項，修正中」。
- 設定：專案的「由 server 執行驗收」開關；禁止清單編輯（全域與專案兩個入口，管理員才看得到編輯）。
- 桌面本機專案的驗收者畫面不動。

## 9. 錯誤處理

| 情況 | 處理 |
|---|---|
| Claude Code 不可用 | 驗收卡不領取，留在待派送並顯示原因（同現有編排） |
| 沒有 Node／瀏覽器 | 提示要求相關條件標 `manual`，不能當通過 |
| 額度用完 | 放回等待，重置後用 `--resume` 接續（沿用現有機制） |
| 驗收中 server 重啟 | 結束孤兒 claude 與佔埠行程，放回待派送，重新驗收（接續對話） |
| 整合 worktree 合併衝突 | 驗收卡失敗，原因寫明哪個分支；管理員處理後「重新驗收」 |
| `verify.json` 缺或壞 | 驗收卡失敗；可重新驗收 |
| 驗收卡結束 | 關伺服器與瀏覽器、清埠、刪整合 worktree、清對話檔 |
| 證據檔路徑穿越 | 拒絕（只接受證據資料夾底下的單一檔名） |

## 10. 成本

網頁驗收每步都回頁面結構，一輪估計一兩百萬 token。驗收者可在角色設定指定較便宜的模型（已有功能）。`--snapshot-mode`、`--mobile` 等省 token 選項先不開，有實際用量數字再調。

## 11. 測試與驗收

### 11.1 自動測試（假 claude，不花額度）

- core 狀態機（純函式）：`vfix` 不計入 `fixes`、模組 Passed→驗收退回→Waiting→NeedsReview→Passed 的完整鏈、輪數（驗收退回與審查退回各自算）、簽章。**先寫測試再改**。
- `verify.json` 解析：格式錯、沒寫、`verdict` 與 items 不一致、`manual` 沒原因。
- server 整合流程：全過→驗收通過→才有合併卡；退回一輪再通過；找不到模組停住；超過 2 輪停住；放行；重新驗收；沒啟用時流程與現在完全一樣（回歸）。
- 整合 worktree：合併成功、衝突、用完清除。
- 埠清理：假行程監聽指定埠，只結束那 3 個埠。
- 禁止清單：預設內容、全域＋專案覆寫的合併結果、規則確實帶進命令列、被擋命令進報告、格式檢查、稽核紀錄。
- API：權限（成員唯讀、管理員可改）、證據檔路徑穿越被擋。
- 前端：驗收者卡片各狀態、驗收報告、禁止清單編輯、開關。

### 11.2 實機（真實 Claude Code）

1. 故意有串接 bug 的小網頁專案：開瀏覽器點擊、截圖出現在報告、判定退回且 `module_id` 正確。
2. 修好後重新驗收通過，才出現合併卡；管理員核准後合併。
3. 驗收中把 server 強制結束再重啟：沒有殘留的 Claude、瀏覽器、佔埠行程。
4. 故意違規（要求 `git push`）：被禁止清單擋下並顯示在報告。

### 11.3 平台

Windows 與 Linux 的 Chrome 偵測與埠清理沿用桌面版寫法，**無法在這兩個平台實測**，說明文件與 CHANGELOG 要如實註明。

## 12. 里程碑

1. **後端核心**：狀態機、整合 worktree、驗收模式與禁止清單、`verify.json`、埠清理、修正卡與輪數、合併卡閘門、放行與重新驗收、API。全部用假 claude 測試。
2. **報告與畫面**：驗收者卡片、驗收報告（含截圖）、專案開關、禁止清單編輯。
3. **真實 Claude 實機驗收**：§11.2 四個情境，補文件與 CHANGELOG（只寫驗證過的）。

## 13. 不做（第一版）

- 沙箱隔離（容器、獨立使用者）。
- 電腦操作（接管螢幕與滑鼠）。
- 多位驗收者、驗收者自己改程式。
- 在已部署網站上驗收。
- `ask_human`（一律標 `manual`）。
- 成員層級的禁止清單權限（只有管理員能改）。
- 驗收結果的歷史比較、趨勢。
