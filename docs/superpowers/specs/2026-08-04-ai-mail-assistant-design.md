# AI 信箱助理（郵件摘要／語意搜尋／AI 回信）— 設計

日期：2026-08-04
狀態：待使用者核可

## 問題

AITerm 目前是「終端機 + AI 指令輔助」工具，沒有任何信箱相關功能。使用者想透過 AI 自然語言：

1. 定期讀取自己設定的信箱，把新信摘要給他看，重要的信件要主動提醒
2. 用自然語言搜尋信件（例如「某家公司 2026 年的報價」），包含附件內容
3. 針對某封信請 AI 草擬回信，經確認後才送出

這是一個全新的子系統（信箱協定、AI 摘要、語意搜尋、本機資料快取），跟現有的 PTY / AI 指令 / DB 連線是平行的新領域，而非既有功能的延伸。

## 範圍

**含：**
- 多組 IMAP/SMTP 信箱帳號（通用協定，不做特定廠商 OAuth）
- App 開啟期間、每帳號定時輪詢新信，AI 摘要 + 重要性判斷；重要信件跳 OS 通知，其餘只更新側邊欄未讀數字
- 摘要／回信草稿歷史本機持久化（SQLite），App 重啟後仍看得到
- AI 自然語言語意搜尋（embedding 向量 + 關鍵字混合排序），本機找不到時退回即時 IMAP SEARCH
- 附件（PDF/Word/Excel）文字擷取，併入摘要與搜尋索引；其餘類型只記 metadata
- AI 草擬回信 → 使用者確認 → 才送出（沿用 `/ai` 既有的確認閘門模式）
- 回信可夾帶本機檔案附件

**不含（本次範圍外，之後可能的延伸）：**
- App 關閉後仍在背景常駐輪詢（tray icon / OS 服務）
- 特定廠商 OAuth 登入（Gmail API、Microsoft Graph 等），僅通用 IMAP/SMTP + app password
- 附件內容的 OCR（圖片文字辨識）
- 使用者自訂「重要信件」規則（VIP 寄件人清單、關鍵字規則）；重要性完全交給 AI 判斷
- 惡意附件的沙箱隔離解析（只做唯讀文字擷取，不額外沙箱）

## 範圍過大，建議分期實作

這個功能涵蓋多帳號輪詢、通知、持久化、語意搜尋、附件解析、AI 回信六個環節，單一實作計畫難以一次做完，寫實作計畫時建議依此順序分期，每期都是可獨立驗證、可先上線的完整切片：

1. 帳號管理 + 輪詢 + AI 摘要 + 重要性通知（不含搜尋、附件解析）
2. 摘要/草稿歷史本機持久化（SQLite）
3. 附件（PDF/Word/Excel）文字擷取，併入摘要
4. AI 語意搜尋（含 IMAP SEARCH fallback）
5. AI 草擬回信 + 確認送出

## 架構總覽

新增後端模組 `src-tauri/src/mail/`，比照 `pty/`、`ai/`、`db/` 的組織方式：每個信箱帳號對應一個 tokio 背景任務，由後端擁有生命週期（App 啟動或帳號新增時 spawn，帳號移除或 App 結束時任務自然結束），透過 Tauri event 把結果推給前端 —— 這跟 `pty://data/{sessionId}`、`ai-stream` 的既有事件模式一致，前端只需訂閱事件，不用自己管 timer。

語意搜尋直接重用知識庫功能已經寫好的向量搜尋管線（`knowledge_base/embedding.rs`、`chunk.rs`，以及 `db/knowledge_base.rs::search_similar_chunks` 的暴力法 cosine + 關鍵字加權演算法），只是把資料表換成信件專用的 `mail_messages` / `mail_chunks`，不新增向量資料庫依賴。

## 元件拆解

### 後端 `src-tauri/src/mail/`

| 檔案 | 職責 |
|---|---|
| `client.rs` | IMAP/SMTP 連線封裝：抓信（`BODY.PEEK`，不動 `\Seen` 避免影響使用者其他信箱用戶端的已讀狀態）、抓附件、SEARCH、寄信 |
| `poller.rs` | 每帳號一個背景任務：定時抓新信 → 附件下載＋文字擷取 → AI 摘要＋重要性判斷（本文＋附件文字一起送進 prompt）→ 寫 SQLite → 發 `mail://summary/{account_id}`、重要信另發 `mail://important/{account_id}` |
| `attachment.rs` | 附件落地存檔（App data 目錄）＋文字擷取（PDF/DOCX/XLSX），失敗或超過大小上限只記 metadata |
| `search.rs` | 自然語言查詢 → embed → 本機 `mail_chunks` 暴力法 cosine＋關鍵字加權；分數過低時用 AI 抽取關鍵字/日期範圍，退回即時 IMAP SEARCH，找到後立刻快取＋切塊＋embed |
| `store.rs` | SQLite 存取層，沿用現有 `db/` 的 SQLite adapter |
| `commands.rs` | 曝露給前端的 `#[tauri::command]`（見下方 IPC 清單） |

**憑證**：帳號的 host/port/使用者名稱等非機密設定走現有 `config/` JSON store；密碼/app password 走現有 OS keyring 封裝，一帳號一組 key（沿用 AI provider API key 現有模式）。移除帳號時一併刪除對應 keyring 項目與所有快取資料（cascade）。

**Embedding provider**：語意搜尋需要一個 embedding provider，沿用知識庫「建立前先探測驗證」的模式；預設帶入使用者已設定給某個知識庫筆記本的那組，也可另外指定。沒有任何可用 provider 時，搜尋功能停用，但摘要/輪詢/回信等其他功能不受影響。

### 前端 `src/`

| 元件 | 職責 |
|---|---|
| `components/Mail/MailPanel.tsx` | 新側邊欄面板（比照 Settings/知識庫）：帳號切換器、信件列表（未讀數字）、搜尋框、摘要/回信草稿檢視 |
| `components/Mail/MailAccountSettings.tsx` | 新增/編輯/刪除信箱帳號表單（比照 Provider 設定表單）：host/port/帳密、輪詢間隔（預設 300 秒，可調）、選擇 embedding provider |
| `components/Mail/MailDetailView.tsx` | 點選信件後顯示完整原文（含附件清單，可下載/開啟） |
| `components/Mail/ComposeReplyModal.tsx` | AI 草擬回信、可編輯，Send 前走確認閘門（比照 `CommandPreview`） |
| `hooks/useMailAccounts.ts` | 帳號 CRUD、訂閱輪詢事件 |
| `hooks/useMailSearch.ts` | 呼叫 `mail_search`，管理搜尋結果狀態 |

重要信件觸發時呼叫 Tauri notification API 跳出 OS 通知；一般未讀只更新側邊欄圖示上的未讀數字。

## 資料模型（SQLite，沿用 `db/` 現有 SQLite adapter）

```
mail_accounts(id, email, imap_host, imap_port, smtp_host, smtp_port, username,
              poll_interval_secs DEFAULT 300, embedding_provider_id NULL,
              created_at)
mail_messages(id, account_id FK, uid, sender, subject, date, body_text,
              ai_summary, is_important BOOLEAN, is_read_locally BOOLEAN DEFAULT 0,
              fetched_at)
mail_chunks(id, message_id FK, source ENUM('body','attachment'), source_filename NULL,
            text, embedding BLOB)
mail_attachments(id, message_id FK, filename, mime_type, size_bytes,
                  local_file_path, parse_status ENUM('unparsed','parsed',
                  'skipped_too_large','unsupported_type','parse_failed'))
mail_reply_drafts(id, message_id FK, draft_text, created_at, sent_at NULL)
```

已讀狀態（`is_read_locally`）只在本機追蹤，不寫回伺服器 `\Seen`——避免使用者在手機或其他信箱用戶端看到「被 AI 讀過」的已讀標記。側邊欄未讀數字＝所有帳號 `is_read_locally = 0` 的筆數。

## 資料流程

**輪詢**：`poller.rs` 定時任務 → IMAP 抓新信（`BODY.PEEK`）→ 有附件則下載＋文字擷取（`attachment.rs`）→ AI 摘要＋重要性判斷（本文＋附件文字）→ 寫 `mail_messages`＋切塊 embed 寫 `mail_chunks`＋附件 metadata 寫 `mail_attachments` → 發事件。重要 → 額外發 `mail://important/{account_id}`，前端跳 OS 通知；否則前端只更新未讀數字。

**搜尋**：使用者在 `MailPanel` 輸入自然語言 → `mail_search` → embed 查詢 → 本機 `mail_chunks` cosine+關鍵字比對 top-K，同信取最高分片段代表 → 分數過低 → AI 抽關鍵字/日期範圍 → IMAP SEARCH → 找到即快取入庫 → 回傳結果（寄件人/主旨/日期/既有 AI 摘要/命中片段）→ 前端列表；點選 → `MailDetailView` 顯示 `mail_messages.body_text` 全文與附件清單。

**回信**：使用者對某封信按「AI 回信」→ 可附加指示（如「婉拒」「確認時間」）→ AI 生成草稿存入 `mail_reply_drafts` → `ComposeReplyModal` 顯示可編輯 → 使用者按送出前走確認閘門（比照 `CommandPreview` 的 risk 確認 UI，寄出郵件視為必須手動確認的動作，絕不自動送出）→ 確認後 `mail_send_reply` 走 SMTP，正確帶 `In-Reply-To`/`References` header 維持信件串；可選擇夾帶本機檔案。

## IPC 指令清單

`mail_add_account` / `mail_remove_account` / `mail_list_accounts` / `mail_list_messages(account_id)` / `mail_search(query, account_id?)` / `mail_get_message(message_id)` / `mail_draft_reply(message_id, instructions?)` / `mail_send_reply(message_id, draft_text, attachments?)` / `mail_mark_read(message_id)`

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

**安全邊界**：附件只做唯讀文字擷取，不執行巨集、不執行附件本身；PDF/Office 解析器本身偶有安全漏洞，這是所有處理附件的信箱軟體都有的既有風險，本次不額外做沙箱隔離。寄出郵件一律需要使用者手動確認，沒有任何路徑會自動送出。信件全文與附件明碼存在本機 SQLite/檔案系統，與現有知識庫文件的儲存方式一致，不做額外加密。

## 測試

Rust（`src-tauri/tests/`，wiremock 模擬 IMAP/SMTP 與 AI provider）：
- 輪詢：新信抓取→摘要→重要性判斷→事件觸發的整條流程
- 單一帳號連線失敗不影響其他帳號輪詢
- 附件大小超限/類型不支援時只記 metadata，不阻斷信件摘要
- 搜尋：本機 cosine+關鍵字排序正確性；分數過低時觸發 IMAP SEARCH fallback
- 移除帳號時 cascade 刪除與 keyring 清除
- 寄信失敗時草稿不被標記為已送出

前端（Vitest + React Testing Library）：
- `MailPanel` 未讀數字與重要信通知的觸發時機
- `ComposeReplyModal` 送出前必須經過確認閘門，不能一按就送
- 搜尋結果點選後 `MailDetailView` 正確顯示全文與附件

依 TDD，每個測試都要先確認會紅再寫實作。

## 已考慮並否決的方案

**輪詢由前端 `setInterval` 驅動，後端只提供單次 fetch/summarize command。** 實作較簡單，但多帳號時前端要自己管多組 timer 與並行 invoke，且與現有「後端擁有狀態、前端訂閱事件」的架構風格（`pty/`、`ai/`）不一致，可靠度也較差。否決。

**搜尋只做 AI 關鍵字抽取 + 既有 FTS，不做向量語意搜尋。** 實作更輕、不需要使用者設定 embedding provider，但對語意/情緒類查詢召回率較差；使用者明確要求語意搜尋能力，且知識庫既有的向量管線可直接重用，成本不高。否決。

**附件只顯示 metadata，不解析內容。** 最簡單、無安全疑慮，但使用者的核心情境（報價單）很常見是 PDF 附件而非信件本文，不解析會讓搜尋直接找不到，違背功能初衷。否決。

**App 關閉後仍背景常駐輪詢。** 體驗更即時，但需要 tray icon 或 OS 服務等跨三平台的常駐機制，複雜度與維護成本大幅上升，且使用者已確認「只在 App 開啟時輪詢」即可。不在本次範圍，列為未來可能的延伸。

## 這個設計不保證什麼

**語意搜尋的召回範圍受限於「已輪詢過的信」。** 伺服器上還沒被輪詢抓過的舊信，語意向量並不存在，只能靠關鍵字 IMAP SEARCH 退回機制找到，且找到後才會補建索引——第一次搜到某封舊信時，語意比對其實還沒發生。

**「重要信件」的判斷完全交給 AI，沒有使用者可調的規則或門檻。** 這代表判斷標準不可預測、也無法保證跟使用者的主觀認定一致；如果誤判過多或過少，本次設計沒有提供使用者側的調整機制（只能靠之後的疊代改 prompt）。

**附件文字擷取的品質取決於檔案本身的結構。** 掃描版 PDF（純圖片、沒有文字層）擷取不到任何文字，會被視同「沒有可用內容」，摘要與搜尋都無法涵蓋——這不是解析失敗，而是這次範圍明確不含 OCR 的直接後果。
