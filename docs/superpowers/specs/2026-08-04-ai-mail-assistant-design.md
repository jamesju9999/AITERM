# AI 信箱助理（郵件摘要／語意搜尋／AI 回信）— 設計

日期：2026-08-04
狀態：待使用者核可

## 問題

AITerm 目前是「終端機 + AI 指令輔助」工具，沒有任何信箱相關功能。使用者想透過 AI 自然語言：

1. 定期讀取自己設定的信箱，把新信摘要給他看，重要的信件要主動提醒
2. 用自然語言搜尋信件（例如「某家公司 2026 年的報價」），包含附件內容
3. 針對某封信請 AI 草擬回信，經確認後才送出
4. 用自然語言請 AI 找出廣告/行銷信，列出候選清單讓使用者確認（可排除誤判）後再刪除

這是一個全新的子系統（信箱協定、AI 摘要、語意搜尋、本機資料快取），跟現有的 PTY / AI 指令 / DB 連線是平行的新領域，而非既有功能的延伸。

## 範圍

**含：**
- 多組 IMAP/SMTP 信箱帳號（通用協定，不做特定廠商 OAuth）
- App 開啟期間、每帳號一條長連線，新信由伺服器即時推播（IMAP IDLE，RFC 2177；伺服器不支援才退回定時輪詢），AI 摘要 + 重要性判斷；重要信件跳 OS 通知，其餘只更新分頁圖示上的未讀數字
- 摘要／回信草稿歷史本機持久化（SQLite），App 重啟後仍看得到
- AI 自然語言語意搜尋（embedding 向量 + 關鍵字混合排序），本機找不到時退回即時 IMAP SEARCH
- 附件（PDF/Word/Excel）文字擷取，併入摘要與搜尋索引；其餘類型只記 metadata
- AI 草擬回信 → 使用者確認 → 才送出（沿用 `/ai` 既有的確認閘門模式）
- 回信可夾帶本機檔案附件
- 自然語言觸發廣告信清理：AI 從已輪詢過的信中列出候選廣告信 → 使用者可勾除誤判 → 確認後移到伺服器 Trash 資料夾（非永久刪除）

**不含（本次範圍外，之後可能的延伸）：**
- App 關閉後仍在背景常駐輪詢（tray icon / OS 服務）
- 特定廠商 OAuth 登入（Gmail API、Microsoft Graph 等），僅通用 IMAP/SMTP + app password
- 附件內容的 OCR（圖片文字辨識）
- 使用者自訂「重要信件」/「廣告信」規則（VIP 寄件人清單、關鍵字規則）；分類完全交給 AI 判斷
- 惡意附件的沙箱隔離解析（只做唯讀文字擷取，不額外沙箱）
- 對尚未輪詢過的舊信做廣告信掃描（避免對整個信箱歷史逐封發 AI 分類請求，成本與延遲過高）
- 廣告信永久刪除／清空垃圾桶（本次只做移到 Trash；永久清除交給信箱伺服器自己的垃圾桶保留規則）

## 範圍過大，建議分期實作

這個功能涵蓋多帳號輪詢、通知、持久化、語意搜尋、附件解析、AI 回信、廣告信清理七個環節，單一實作計畫難以一次做完，寫實作計畫時建議依此順序分期，每期都是可獨立驗證、可先上線的完整切片：

1. 帳號管理 + 輪詢 + AI 摘要 + 重要性判斷 + 通知（不含搜尋、附件解析）
2. 摘要/草稿歷史本機持久化（SQLite）
3. 附件（PDF/Word/Excel）文字擷取，併入摘要
4. AI 語意搜尋（含 IMAP SEARCH fallback）
5. AI 草擬回信 + 確認送出
6. 廣告信偵測（併入第 1 期的 AI 分類 prompt）+ 清單確認 + 移到 Trash

## 架構總覽

新增後端模組 `src-tauri/src/mail/`（domain 邏輯）＋ `src-tauri/src/commands/mail.rs`（`#[tauri::command]` 進入點）＋ `src-tauri/src/db/mail.rs`（SQLite 存取），比照既有 `knowledge_base`/`db`/`ai` 的檔案組織慣例。每個信箱帳號對應一個 tokio 背景任務，由 `MailState { tasks: HashMap<account_id, JoinHandle<()>> }`（`tokio::sync::Mutex` 包住，比照 `telegram::TelegramState` 的 spawn/abort 模式）管理生命週期：帳號新增時 spawn、帳號移除或任務要重啟時先 `.abort()` 舊的再 spawn 新的，App 結束時任務隨行程結束。輪詢結果透過固定名稱的 Tauri event `mail-sync-event`（payload 帶 `kind` 標籤與 `account_id`）推給前端，比照既有 `ai-stream`、`kb-sync-event` 的扁平事件模式；前端只需訂閱事件，不用自己管 timer。

語意搜尋直接重用知識庫功能已經寫好的向量搜尋管線（`knowledge_base/embedding.rs`、`chunk.rs`，以及 `db/knowledge_base.rs::search_similar_chunks` 的暴力法 cosine + 關鍵字加權演算法），只是把資料表換成信件專用的 `mail_messages` / `mail_chunks`，不新增向量資料庫依賴。

## 元件拆解

後端檔案分三處，比照 `knowledge_base`/`db`/`commands` 的既有分工（domain 邏輯與 SQLite 存取分開、`#[tauri::command]` 進入點集中在 `commands/`，而非像 `pty` 那樣把 commands 收在自己資料夾內）：

| 檔案 | 職責 |
|---|---|
| `src-tauri/src/mail/client.rs` | IMAP/SMTP 連線封裝：抓信（`BODY.PEEK`，不動 `\Seen` 避免影響使用者其他信箱用戶端的已讀狀態）、抓附件、SEARCH、寄信 |
| `src-tauri/src/mail/poller.rs` | 每帳號一個背景任務，維持一條長連線並停在 IMAP IDLE 等伺服器推播（伺服器不支援 IDLE 才退回 `poll_interval_secs` 定時輪詢；連線斷掉以指數退避重連，上限為該間隔）：抓新信 → 附件下載＋文字擷取 → AI 摘要＋重要性判斷＋廣告信判斷（同一次 AI 呼叫的結構化輸出，本文＋附件文字一起送進 prompt）→ 寫 SQLite → 發固定名稱事件 `mail-sync-event`（payload 用 `kind` 欄位區分 `summary`/`important`，並帶 `account_id`——比照現有 `ai-stream`、`kb-sync-event` 的扁平事件模式，而非 PTY 那種每個 session 一個獨立事件名稱的 URI 命名法，因為信件同步不像終端機輸出那樣是高頻逐字元串流） |
| `src-tauri/src/mail/attachment.rs` | 附件落地存檔（App data 目錄）＋文字擷取（PDF/DOCX/XLSX），失敗或超過大小上限只記 metadata |
| `src-tauri/src/mail/search.rs` | 自然語言查詢 → embed → 本機 `mail_chunks` 暴力法 cosine＋關鍵字加權；分數過低時用 AI 抽取關鍵字/日期範圍，退回即時 IMAP SEARCH，找到後立刻快取＋切塊＋embed |
| `src-tauri/src/mail/cleanup.rs` | 從 `mail_messages` 撈 `is_promotional = 1` 且尚未處理的候選信（可套用自然語言解析出的日期/帳號篩選），確認後對選中信件執行 IMAP MOVE 到 Trash 資料夾，並同步清掉本機快取 |
| `src-tauri/src/db/mail.rs` | SQLite 存取層（`mail.db`），沿用現有 `db/knowledge_base.rs` 的 `SqlitePool` + 手寫 `CREATE TABLE IF NOT EXISTS` 模式 |
| `src-tauri/src/commands/mail.rs` | 曝露給前端的 `#[tauri::command]`（見下方 IPC 清單），比照 `commands/knowledge_base.rs` |

**憑證**：帳號的 host/port/使用者名稱等非機密設定走現有 `config/` 的 `AppConfig.mail_accounts: Vec<MailAccountConfig>`（比照既有 `db_connections`/`vcs_connections` 模式）；密碼/app password 走現有 OS keyring 封裝，一帳號一組 key（沿用既有 `SecretStore` 模式）。移除帳號時一併刪除對應 keyring 項目與所有快取資料（cascade）。

**Embedding provider**：語意搜尋需要一個 embedding provider，沿用知識庫「建立前先探測驗證」的模式；預設帶入使用者已設定給某個知識庫筆記本的那組，也可另外指定。沒有任何可用 provider 時，搜尋功能停用，但摘要/輪詢/回信等其他功能不受影響。

### 前端 `src/`

AITerm 沒有「可切換的側邊欄功能面板」這種模型：每個大功能是透過 `NewTabPicker` 開的一種**分頁類型**（比照現有 `"knowledge-base"` 分頁類型 + `KnowledgeBaseView`），分頁切換靠 CSS visibility 而非 mount/unmount；跟帳密相關的設定則走獨立的 Settings 路由（比照 `DatabaseConnectionsPage`/`VcsConnectionsPage`）。

| 元件 | 職責 |
|---|---|
| `src/components/MailView/MailView.tsx` | 新分頁類型 `"mail"` 的內容（`TabBar`/`NewTabPicker`/`TerminalApp.tsx` 三處各加一筆註冊，比照知識庫分頁）：帳號切換器、信件列表（未讀數字）、摘要檢視 |
| `src/components/Settings/MailAccountsPage.tsx` | 新增/編輯/刪除信箱帳號表單（比照 `DatabaseConnectionsPage`），掛在 `SettingsView.tsx` 新增的 `"mail"` sidebar 分頁下：host/port/帳密、輪詢間隔（預設 300 秒，可調）、選擇 embedding provider |
| `src/components/MailView/MailDetailView.tsx` | 點選信件後顯示完整原文（含附件清單，可下載/開啟） |
| `src/components/MailView/ComposeReplyModal.tsx` | AI 草擬回信、可編輯，Send 前走確認閘門（比照 `CommandPreview`） |
| `src/components/MailView/MailCleanupConfirm.tsx` | 顯示 AI 找到的廣告信候選清單，每筆預設勾選、使用者可取消勾選排除誤判，確認後才執行刪除（比照 `CommandPreview` 的批次版本） |
| `src/hooks/useMailSync.ts` | 訂閱 `mail-sync-event`，管理帳號/信件列表狀態（比照 `useAiChat.ts` 的 `mountedRef` + `active` 雙重保護寫法） |
| `src/hooks/useMailSearch.ts` | 呼叫 `mail_search`，管理搜尋結果狀態 |

重要信件觸發時呼叫 `tauri-plugin-notification`（目前專案未安裝，需新增）跳出 OS 通知；一般未讀只更新分頁圖示上的未讀數字。

## 資料模型（SQLite，沿用 `db/` 現有 SQLite adapter）

帳號的 host/port/使用者名稱/輪詢間隔/embedding provider 等**非機密設定存在 `config/` 的 `AppConfig.mail_accounts: Vec<MailAccountConfig>`**（比照既有的 `db_connections`/`vcs_connections` 模式，同樣提供 `add_mail_account`/`update_mail_account`/`remove_mail_account`），密碼/app password 存 OS keyring；不進 SQLite，避免同一份帳號設定存在兩個地方。SQLite（`mail.db`）只放信件資料與輪詢運作狀態：

```
mail_poll_state(account_id PRIMARY KEY, last_seen_uid INTEGER, last_polled_at)
mail_messages(id, account_id, uid, sender, subject, date, body_text,
              ai_summary, is_important BOOLEAN, is_promotional BOOLEAN,
              is_read_locally BOOLEAN DEFAULT 0, fetched_at)
mail_chunks(id, message_id FK, source ENUM('body','attachment'), source_filename NULL,
            text, embedding BLOB)
mail_attachments(id, message_id FK, filename, mime_type, size_bytes,
                  local_file_path, parse_status ENUM('unparsed','parsed',
                  'skipped_too_large','unsupported_type','parse_failed'))
mail_reply_drafts(id, message_id FK, draft_text, created_at, sent_at NULL)
```

已讀狀態（`is_read_locally`）只在本機追蹤，不寫回伺服器 `\Seen`——避免使用者在手機或其他信箱用戶端看到「被 AI 讀過」的已讀標記。Mail 分頁圖示上的未讀數字＝所有帳號 `is_read_locally = 0` 的筆數。

## 資料流程

**收信**：`poller.rs` 連上 IMAP 後先同步一次（補齊斷線期間的信，並比對伺服器端刪除），接著停在 IDLE 等推播。IDLE 只會說「有東西變了」而不會說是哪一封，所以收到通知就跑一次同步；`poll_interval_secs` 有兩個用途：一是 IDLE 的存活檢查上限，避免筆電睡眠或 NAT 斷線讓連線靜默死掉，二是刪除比對（`UID SEARCH ALL`，大信箱很重）的最小間隔——推播通知本身不做比對，但距離上次比對超過一個間隔就補做一次。同步內容：IMAP 抓新信（`BODY.PEEK`）→ 有附件則下載＋文字擷取（`attachment.rs`）→ AI 摘要＋重要性判斷＋廣告信判斷（本文＋附件文字，同一次 AI 呼叫的結構化輸出）→ 寫 `mail_messages`＋切塊 embed 寫 `mail_chunks`＋附件 metadata 寫 `mail_attachments` → 發 `mail-sync-event`（`kind: "summary"`）。重要 → 額外發一筆 `kind: "important"` 的 `mail-sync-event`，前端跳 OS 通知；否則前端只更新未讀數字。

**搜尋**：使用者在 `MailView` 輸入自然語言 → `mail_search` → embed 查詢 → 本機 `mail_chunks` cosine+關鍵字比對 top-K，同信取最高分片段代表 → 分數過低 → AI 抽關鍵字/日期範圍 → IMAP SEARCH → 找到即快取入庫 → 回傳結果（寄件人/主旨/日期/既有 AI 摘要/命中片段）→ 前端列表；點選 → `MailDetailView` 顯示 `mail_messages.body_text` 全文與附件清單。

**回信**：使用者對某封信按「AI 回信」→ 可附加指示（如「婉拒」「確認時間」）→ AI 生成草稿存入 `mail_reply_drafts` → `ComposeReplyModal` 顯示可編輯 → 使用者按送出前走確認閘門（比照 `CommandPreview` 的 risk 確認 UI，寄出郵件視為必須手動確認的動作，絕不自動送出）→ 確認後 `mail_send_reply` 走 SMTP，正確帶 `In-Reply-To`/`References` header 維持信件串；可選擇夾帶本機檔案。

**廣告信清理**：使用者用自然語言觸發（例如在 `/ai` 或 Mail 面板輸入「幫我清掉廣告信」）→ `mail_list_promotional_candidates`：可選擇性用 AI 從查詢中抽取帳號/日期範圍篩選（重用 `search.rs` 的關鍵字/日期抽取邏輯），撈出對應帳號中 `is_promotional = 1` 的信 → 前端 `MailCleanupConfirm` 顯示候選清單（寄件人/主旨/日期/摘要），每筆預設勾選 → 使用者可取消勾選排除誤判 → 按確認 → `mail_delete_messages(message_ids)`：對每筆信執行 IMAP MOVE 到該帳號的 Trash 資料夾（伺服器不支援 MOVE 則退回 COPY + STORE `\Deleted` + EXPUNGE），成功後同步從 `mail_messages`/`mail_chunks`/`mail_attachments` 移除該筆快取。`is_promotional` 判斷本身在輪詢階段就已算好（與 `is_important` 同一次 AI 呼叫），觸發清理時不需要重新呼叫 AI 分類，只是把已有的分類結果拿出來給使用者確認。

## IPC 指令清單

`mail_add_account` / `mail_remove_account` / `mail_list_accounts` / `mail_list_messages(account_id)` / `mail_search(query, account_id?)` / `mail_get_message(message_id)` / `mail_draft_reply(message_id, instructions?)` / `mail_send_reply(message_id, draft_text, attachments?)` / `mail_mark_read(message_id)` / `mail_list_promotional_candidates(query?, account_id?)` / `mail_delete_messages(message_ids)`

## 錯誤處理

| 情境 | 行為 |
|---|---|
| IMAP/SMTP 連線失敗（帳密錯、網路斷） | 該次輪詢跳過，記錄錯誤狀態，下次輪詢正常重試；不讓單一帳號故障影響其他帳號的輪詢任務 |
| 附件超過大小上限 | 只記 metadata（`skipped_too_large`），不下載內容、不解析 |
| 附件類型不支援或解析失敗 | 只記 metadata（`unsupported_type` / `parse_failed`），不中斷該封信的摘要流程（用信件本文照常摘要） |
| 搜尋時沒有可用 embedding provider | 搜尋功能停用（UI 顯示需先設定），不影響摘要/輪詢/回信 |
| 本機語意搜尋分數過低 | 自動退回 AI 關鍵字抽取 + IMAP SEARCH，找到後即時補建索引 |
| AI 摘要/草擬回信呼叫失敗 | 沿用既有 `AiError` 錯誤分類（`network` / `rate_limit` / `model_error` 等），該封信摘要留空並標記失敗，使用者可手動重試 |
| 寄信失敗（SMTP 錯誤） | 草稿保留在 `mail_reply_drafts`，不視為已送出，UI 顯示錯誤並可重試 |
| 移除帳號 | cascade 刪除該帳號所有 `mail_messages`/`mail_chunks`/`mail_attachments`/`mail_reply_drafts`，並清除 keyring 憑證 |
| 該信箱沒有 Trash 資料夾、且不支援 MOVE/COPY 退回機制 | 整批清理直接回錯誤，不執行任何刪除，UI 提示使用者該信箱不支援；絕不因為找不到 Trash 就改成永久刪除 |
| 清理批次中部分信件刪除失敗（例如已被其他用戶端搬走） | 該筆略過並在結果中標示失敗，其餘照常執行完，不整批中止 |

**安全邊界**：附件只做唯讀文字擷取，不執行巨集、不執行附件本身；PDF/Office 解析器本身偶有安全漏洞，這是所有處理附件的信箱軟體都有的既有風險，本次不額外做沙箱隔離。寄出郵件、刪除信件都一律需要使用者手動確認（廣告信清理走「候選清單勾選＋確認」，不存在自動刪除路徑），沒有任何路徑會自動送出或自動刪除。信件全文與附件明碼存在本機 SQLite/檔案系統，與現有知識庫文件的儲存方式一致，不做額外加密。

## 測試

Rust（`src-tauri/tests/`，wiremock 模擬 IMAP/SMTP 與 AI provider）：
- 輪詢：新信抓取→摘要→重要性判斷→事件觸發的整條流程
- 單一帳號連線失敗不影響其他帳號輪詢
- 附件大小超限/類型不支援時只記 metadata，不阻斷信件摘要
- 搜尋：本機 cosine+關鍵字排序正確性；分數過低時觸發 IMAP SEARCH fallback
- 移除帳號時 cascade 刪除與 keyring 清除
- 寄信失敗時草稿不被標記為已送出
- 廣告信清理：候選清單只來自 `is_promotional = 1` 的信；沒有 Trash 資料夾時整批回錯誤且不刪除任何信；批次中單筆刪除失敗不影響其他筆

前端（Vitest + React Testing Library）：
- `MailView` 未讀數字與重要信通知的觸發時機
- `ComposeReplyModal` 送出前必須經過確認閘門，不能一按就送
- 搜尋結果點選後 `MailDetailView` 正確顯示全文與附件
- `MailCleanupConfirm` 預設全選、取消勾選後該筆不會出現在送出的 `message_ids` 裡、沒有勾選任何一筆時確認按鈕為 disabled

依 TDD，每個測試都要先確認會紅再寫實作。

## 已考慮並否決的方案

**輪詢由前端 `setInterval` 驅動，後端只提供單次 fetch/summarize command。** 實作較簡單，但多帳號時前端要自己管多組 timer 與並行 invoke，且與現有「後端擁有狀態、前端訂閱事件」的架構風格（`pty/`、`ai/`）不一致，可靠度也較差。否決。

**搜尋只做 AI 關鍵字抽取 + 既有 FTS，不做向量語意搜尋。** 實作更輕、不需要使用者設定 embedding provider，但對語意/情緒類查詢召回率較差；使用者明確要求語意搜尋能力，且知識庫既有的向量管線可直接重用，成本不高。否決。

**附件只顯示 metadata，不解析內容。** 最簡單、無安全疑慮，但使用者的核心情境（報價單）很常見是 PDF 附件而非信件本文，不解析會讓搜尋直接找不到，違背功能初衷。否決。

**App 關閉後仍背景常駐輪詢。** 體驗更即時，但需要 tray icon 或 OS 服務等跨三平台的常駐機制，複雜度與維護成本大幅上升，且使用者已確認「只在 App 開啟時輪詢」即可。不在本次範圍，列為未來可能的延伸。

**AI 判定為廣告信就直接刪除，不經使用者確認。** 少一道操作、體驗更順，但刪信是難以復原的動作，AI 分類一定會有誤判，沒有確認步驟等於把誤判的代價完全轉嫁給使用者且無法補救。否決，一律先列候選清單讓使用者勾選確認。

**清理時對整個信箱歷史（含尚未輪詢過的舊信）逐封掃描分類。** 涵蓋範圍更完整，但對大信箱會是大量 AI 呼叫、耗時且成本高，且使用者已確認「只處理已輪詢過的信」即可。不在本次範圍。

## 這個設計不保證什麼

**語意搜尋的召回範圍受限於「已輪詢過的信」。** 伺服器上還沒被輪詢抓過的舊信，語意向量並不存在，只能靠關鍵字 IMAP SEARCH 退回機制找到，且找到後才會補建索引——第一次搜到某封舊信時，語意比對其實還沒發生。

**「重要信件」與「廣告信」的判斷完全交給 AI，沒有使用者可調的規則或門檻。** 這代表判斷標準不可預測、也無法保證跟使用者的主觀認定一致；如果誤判過多或過少，本次設計沒有提供使用者側的調整機制（只能靠之後的疊代改 prompt，或靠清理流程裡的手動勾選排除來補救單次誤判）。

**廣告信清理涵蓋不到尚未輪詢過的舊信。** 跟語意搜尋一樣，`is_promotional` 只在輪詢當下算過，本次範圍明確不做「回頭掃描整個信箱歷史」，所以清理功能天生只對「AITerm 開始監控之後收到的信」有效。

**附件文字擷取的品質取決於檔案本身的結構。** 掃描版 PDF（純圖片、沒有文字層）擷取不到任何文字，會被視同「沒有可用內容」，摘要與搜尋都無法涵蓋——這不是解析失敗，而是這次範圍明確不含 OCR 的直接後果。
