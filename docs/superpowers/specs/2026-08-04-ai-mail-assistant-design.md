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
- 使用者可對任一封已同步的信手動觸發刪除（二次確認），移到伺服器 Trash——不限廣告信，也不需要透過廣告信清理流程；這是第 1 期實際新增的能力，也是整個信箱功能第一個會寫入伺服器狀態的操作（見下方「這個設計不保證什麼」）
- 新增帳號前會先驗證帳密／連線可用才儲存；帳號連線異常會在畫面上提示，不是只寫進 log

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

**Phase 1 已交付，範圍比原計畫大**——完整細節見下面各節，這裡只列跟原計畫不同的地方：

- 遞送機制不是「定時輪詢」，而是每帳號一條長連線、伺服器端 IMAP IDLE 推播；`poll_interval_secs` 降級為連線存活檢查間隔 + 不支援 IDLE 時的退回輪詢間隔。
- 多做了三件原計畫沒設計到的事：伺服器端刪除/搬移同步（`UID SEARCH ALL` 比對）、UIDVALIDITY 追蹤（信箱重建時本機快取歸零重來）、連線健康狀態的 UI 提示（斷線/恢復才提示一次，不是每次重試都提示）。
- 多做了單封信手動刪除到 Trash——這其實是第 6 期廣告信清理才需要的「移到 Trash」原語，Phase 1 為了單封刪除把它先做出來了（`mail/client.rs::move_to_trash`，含 SPECIAL-USE `\Trash` 解析與 MOVE/UIDPLUS 選擇邏輯）。第 6 期實作批次版本時可以直接重用，不必重新設計。
- 新增帳號前多了一道連線驗證（`mail_test_connection`），失敗就不寫入 config/keyring、也不啟動背景任務。
- App 結束、或帳號被移除時，背景任務會優雅收尾（送 IDLE DONE + LOGOUT）而不是直接砍線——這是長連線這個機制本身帶來的必要工作，原計畫沒有預期到。
- 附件解析、SMTP 寄信、語意搜尋、AI 草擬回信、廣告信清單批次確認都還沒做，維持在原計畫的第 3～6 期。

## 架構總覽

新增後端模組 `src-tauri/src/mail/`（domain 邏輯）＋ `src-tauri/src/commands/mail.rs`（`#[tauri::command]` 進入點）＋ `src-tauri/src/db/mail.rs`（SQLite 存取），比照既有 `knowledge_base`/`db`/`ai` 的檔案組織慣例。每個信箱帳號對應一個 tokio 背景任務，由 `MailState { tasks: HashMap<account_id, MailTask> }`（`tokio::sync::Mutex` 包住，比照 `telegram::TelegramState` 的模式，但擴充成 HashMap 因為信箱是多帳號的）管理生命週期。`MailTask` 除了 `JoinHandle` 之外還多帶兩樣東西，都是長連線這個機制本身逼出來的：一個 `watch::Sender<bool>` 用來優雅叫停，一個 `mpsc::Sender<DeleteRequest>` 用來把單封信刪除請求丟給該帳號自己的長連線去做。原計畫設想的是「重啟時 `.abort()` 舊的再 spawn 新的」——這在單次輪詢的舊模型下沒問題，但現在連線幾乎全部時間都停在 IMAP IDLE，`abort()` 會讓 TLS socket 沒送 LOGOUT 就斷掉，佔用伺服器端一個連線槽直到它自己逾時（Gmail 之類的供應商對同帳號的並行 IMAP 連線數有上限，約 15 條）。所以帳號移除、任務重啟、App 結束時實際做的是：翻轉 `watch::Sender`，讓卡在 IDLE 的任務把它讀成一個中斷、送 DONE、LOGOUT、然後才真的結束；給 5 秒，逾時才退回 `abort()` 當最後手段。App 結束時是在 `lib.rs` 的 `.run()` 監聽 `RunEvent::Exit` 觸發 `mail::poller::stop_all`，把所有帳號的任務平行叫停，不會因為某一帳號卡住而拖慢整個 App 結束的時間。

輪詢/同步結果透過固定名稱的 Tauri event `mail-sync-event`（payload 帶 `kind` 標籤與 `account_id`）推給前端，比照既有 `ai-stream`、`kb-sync-event` 的扁平事件模式；前端只需訂閱事件，不用自己管 timer。`kind` 目前有五種，比原計畫的 `summary`/`important` 兩種多了三種：`removed`（本機快取的信因為伺服器端刪除同步或 UIDVALIDITY 重置而消失）、`connection_failed`/`connection_restored`（帳號連線健康狀態的**轉換**——只在剛斷線、剛恢復那一刻各發一次，退避重連期間的每次重試都不會再發，避免把一個正常的重試過程變成洗版的提示）。

語意搜尋直接重用知識庫功能已經寫好的向量搜尋管線（`knowledge_base/embedding.rs`、`chunk.rs`，以及 `db/knowledge_base.rs::search_similar_chunks` 的暴力法 cosine + 關鍵字加權演算法），只是把資料表換成信件專用的 `mail_messages` / `mail_chunks`，不新增向量資料庫依賴。

## 元件拆解

後端檔案分三處，比照 `knowledge_base`/`db`/`commands` 的既有分工（domain 邏輯與 SQLite 存取分開、`#[tauri::command]` 進入點集中在 `commands/`，而非像 `pty` 那樣把 commands 收在自己資料夾內）：

| 檔案 | 職責 |
|---|---|
| `src-tauri/src/mail/client.rs` | IMAP 連線封裝（`MailConnection`）：連線＋登入＋能力探測（是否支援 IDLE、可用哪種安全刪除原語）、同步抓信（`BODY.PEEK`，不動 `\Seen`）、IDLE 等待、`move_to_trash`（唯一會寫入伺服器的操作，MOVE 或 COPY+STORE+UID EXPUNGE，兩者都不支援就拒絕，絕不退回純 EXPUNGE）、`test_connection`。**Phase 1 範圍內尚未實作**：SMTP 寄信、附件下載與文字擷取——這兩塊仍是第 3、5 期的規劃 |
| `src-tauri/src/mail/poller.rs` | 每帳號一個背景任務，維持一條長連線並停在 IMAP IDLE 等伺服器推播（伺服器不支援 IDLE 才退回 `poll_interval_secs` 定時輪詢；連線斷掉以指數退避重連，5 秒起、每次翻倍、上限為該間隔）：抓新信（目前只送本文給 AI，附件文字擷取是第 3 期才有的東西）→ AI 摘要＋重要性判斷＋廣告信判斷（同一次 AI 呼叫的結構化輸出）→ 寫 SQLite → 發 `mail-sync-event`（`kind: summary`/`important`）。另外做三件原計畫沒設計到的事：① 定期用 `UID SEARCH ALL` 跟本機比對，把伺服器端已刪除/搬走的信同步移除本機快取（發 `kind: removed`）；② 追蹤 UIDVALIDITY，信箱被重建時整帳號快取歸零重新同步；③ 連線健康狀態的轉換（不是每次重試）發 `kind: connection_failed`/`connection_restored`。帳號移除或 App 結束時透過 `watch::Sender<bool>` 優雅收尾（送 IDLE DONE + LOGOUT，5 秒逾時才強制中止），而非直接砍連線 |
| `src-tauri/src/mail/manager.rs` | `MailState`/`MailTask`/`DeleteRequest` 型別定義：每帳號一筆 task handle + 優雅叫停用的 `watch::Sender<bool>` + 單封信刪除請求用的 `mpsc::Sender<DeleteRequest>`（原計畫沒有列出這個檔案；長連線 + 單封刪除這兩個能力逼出來的） |
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
| `src/components/MailView/MailView.tsx` | 新分頁類型 `"mail"` 的內容（`TabBar`/`NewTabPicker`/`TerminalApp.tsx` 三處各加一筆註冊，比照知識庫分頁）：帳號切換器、信件列表（AI 摘要用 inline 方式直接顯示在列表裡，目前沒有獨立的詳情頁）、標記已讀、per-帳號連線失敗提示 banner（依 `connection_failed`/`connection_restored` 顯示/清除）、單封信刪除（兩次點擊確認，跟 Settings 移除帳號同一套武裝/確認寫法） |
| `src/components/Settings/MailAccountsPage.tsx` | 新增/編輯/刪除信箱帳號表單（比照 `DatabaseConnectionsPage`），掛在 `SettingsView.tsx` 新增的 `"mail"` sidebar 分頁下：host/port/帳密、輪詢間隔欄位（預設 300 秒，可調；UI 標籤已改成「連線檢查間隔（秒）」以反映實際是 IDLE 推播，底層欄位名 `poll_interval_secs` 沒有跟著改）。存檔前一定先呼叫 `mail_test_connection` 驗證帳密，失敗就整個中止、不寫入 config/keyring、也不啟動背景任務；成功新增帳號後會順便主動要求一次通知權限。**尚未實作**：embedding provider 選擇（語意搜尋是第 4 期才有的東西） |
| `src/components/MailView/MailDetailView.tsx`（尚未實作） | 規劃：點選信件後顯示完整原文（含附件清單，可下載/開啟）——目前信件內容直接用摘要 inline 顯示在列表裡，沒有獨立頁面 |
| `src/components/MailView/ComposeReplyModal.tsx`（尚未實作） | 規劃：AI 草擬回信、可編輯，Send 前走確認閘門（比照 `CommandPreview`）——第 5 期範圍 |
| `src/components/MailView/MailCleanupConfirm.tsx`（尚未實作） | 規劃：顯示 AI 找到的廣告信候選清單，每筆預設勾選、使用者可取消勾選排除誤判，確認後才執行刪除——第 6 期範圍；底層的「移到 Trash」原語已經在第 1 期做出來了 |
| `src/hooks/useMailSync.ts` | 全域掛載一次（在 `TerminalApp.tsx`，不是 `MailView` 裡——這樣即使使用者從沒開過 Mail 分頁也收得到重要信通知）：訂閱 `mail-sync-event`，任何 `kind` 都觸發未讀數重新整理，`kind === "important"` 才額外跳 OS 通知；帳號/信件列表本身的狀態管理是 `MailView.tsx` 自己做的，不在這個 hook 裡。沿用 `mountedRef` 防護寫法 |
| `src/hooks/useMailSearch.ts`（尚未實作） | 規劃：呼叫 `mail_search`，管理搜尋結果狀態——第 4 期範圍 |

重要信件觸發時，`useMailSync.ts` 呼叫 `@tauri-apps/plugin-notification`（`tauri-plugin-notification` 已加入 `Cargo.toml` 並在 `lib.rs` 註冊為 plugin）跳出 OS 通知；一般未讀只更新分頁圖示上的未讀數字。要注意這個通知是**前端**觸發的——後端 poller 本身不呼叫任何通知 API，只發一筆 `kind: "important"` 的 Tauri event，是否跳通知、跳給誰看，完全是前端的決定。

## 資料模型（SQLite，沿用 `db/` 現有 SQLite adapter）

帳號的 host/port/使用者名稱/輪詢間隔/embedding provider 等**非機密設定存在 `config/` 的 `AppConfig.mail_accounts: Vec<MailAccountConfig>`**（比照既有的 `db_connections`/`vcs_connections` 模式，同樣提供 `add_mail_account`/`update_mail_account`/`remove_mail_account`），密碼/app password 存 OS keyring；不進 SQLite，避免同一份帳號設定存在兩個地方。SQLite（`mail.db`）只放信件資料與輪詢運作狀態：

```
mail_poll_state(account_id PRIMARY KEY, last_seen_uid INTEGER,
                 uid_validity INTEGER, last_polled_at)            -- 已實作（uid_validity 是後補的欄位，用來偵測信箱重建）
mail_messages(id, account_id, uid, sender, subject, date, body_text,
              ai_summary, is_important BOOLEAN, is_promotional BOOLEAN,
              is_read_locally BOOLEAN DEFAULT 0, fetched_at)       -- 已實作
mail_chunks(id, message_id FK, source ENUM('body','attachment'), source_filename NULL,
            text, embedding BLOB)                                 -- 規劃中，第 3/4 期才會建
mail_attachments(id, message_id FK, filename, mime_type, size_bytes,
                  local_file_path, parse_status ENUM('unparsed','parsed',
                  'skipped_too_large','unsupported_type','parse_failed'))  -- 規劃中，第 3 期才會建
mail_reply_drafts(id, message_id FK, draft_text, created_at, sent_at NULL) -- 規劃中，第 5 期才會建
```

目前只有 `mail_poll_state` 和 `mail_messages` 兩張表真的存在；其餘三張仍是後續分期的規劃，尚未建立。`uid_validity` 是 Phase 1 過程中加上去的欄位（`init_schema` 用 `PRAGMA table_info` 檢查再視情況 `ALTER TABLE`，讓已裝過舊版的資料庫也能升級），原計畫的資料模型草稿沒有預期到需要它。

已讀狀態（`is_read_locally`）只在本機追蹤，不寫回伺服器 `\Seen`——避免使用者在手機或其他信箱用戶端看到「被 AI 讀過」的已讀標記。Mail 分頁圖示上的未讀數字＝所有帳號 `is_read_locally = 0` 的筆數。

## 資料流程

**收信**：`poller.rs` 為每個帳號維持一條長連線；連上後一定先同步一次（補齊斷線期間的信，並比對伺服器端刪除），接著停在 IDLE 等推播（伺服器不支援 IDLE 才退回 `poll_interval_secs` 定時輪詢）。IDLE 只會說「有東西變了」而不會說是哪一封，所以收到通知就跑一次同步；`poll_interval_secs`（UI 標籤已改叫「連線檢查間隔」，底層欄位名沒變）有兩個用途：一是 IDLE 的存活檢查上限，避免筆電睡眠或 NAT 斷線讓連線靜默死掉，二是刪除比對（`UID SEARCH ALL`，大信箱很重）的最小間隔——單純的 IDLE 推播通知本身不做比對，但距離上次比對超過一個間隔就補做一次；連線剛建立、存活逾時、比對到期時則一定比對。同步內容：SELECT 時順便讀 UIDVALIDITY，跟上次存的不一樣就代表信箱被重建過，整個帳號的快取（`mail_messages`）連同游標（`last_seen_uid`）一起歸零重新同步（並發一筆 `kind: "removed"`）→ IMAP 抓新信（`BODY.PEEK`，目前不下載附件——附件下載/文字擷取是第 3 期才有的東西）→ AI 摘要＋重要性判斷＋廣告信判斷（同一次 AI 呼叫的結構化輸出，目前只送本文；除了 prompt 要求「廣告信絕不算重要」之外，程式碼裡另外強制了這條規則一次，不完全信任模型會照做）→ 寫 `mail_messages` → 發 `mail-sync-event`（`kind: "summary"`）。重要 → 額外發一筆 `kind: "important"` 的 `mail-sync-event`，前端跳 OS 通知；否則前端只更新未讀數字。比對到的伺服器端刪除/搬移一樣發 `kind: "removed"`。

連線斷掉會以指數退避重連（5 秒起、每次翻倍，上限是設定的檢查間隔），連到能撐過大約 10 秒才算「站穩」、退避計數才會歸零。連線健康狀態的**轉換**（不是每次重試）會發 `kind: "connection_failed"`/`"connection_restored"`，Mail 分頁顯示對應提示；純粹的重試過程只寫 log，不打擾使用者也不跳 OS 通知。帳號被移除或 App 結束時，任務會被優雅叫停（送 IDLE DONE + LOGOUT，5 秒內收不到回應才強制中止）——長連線幾乎全部時間停在 IDLE，直接砍連線會讓伺服器端的 session 沒登出乾淨，白白佔用供應商限制的並行連線數（Gmail 約 15 條）。

**刪除（單封信，Phase 1 新增）**：Mail 列表每一列都有刪除按鈕，兩次點擊確認（第一次「武裝」，該列變成可再點一次的危險色確認鈕；滑鼠移出該列會自動解除武裝）。確認後前端呼叫 `mail_delete_message(message_id)`，後端不開新的 IMAP 連線，而是把請求丟進該帳號自己那條長連線的刪除 channel（容量 4），由帳號自己的背景任務在兩次 IDLE 之間（或 fallback 輪詢迴圈裡）服務——避免刪除跟同步互相干擾，也避免多佔一個供應商限制的並行連線數。實際刪除呼叫 `move_to_trash`：伺服器支援 RFC 6851 `MOVE` 就用一次 `UID MOVE`；只支援 RFC 4315 `UIDPLUS` 就退回 `UID COPY` + `UID STORE \Deleted` + `UID EXPUNGE`（`UID EXPUNGE` 只清掉指定 UID，不會動到其他用戶端標過 `\Deleted` 的信）；兩者都不支援就直接回錯誤，**絕不**退回普通 `EXPUNGE`（那會清空整個信箱裡所有被標 `\Deleted` 的信，包含使用者在其他信箱用戶端標的）。Trash 資料夾優先用 RFC 6154 SPECIAL-USE 的 `\Trash` 旗標找，找不到才照固定順序探測名稱（`[Gmail]/Trash`、`Trash`、`INBOX.Trash`）；還是找不到一樣直接回錯誤，不會亂猜亂放。本機那一列只有在伺服器確認刪除成功後才移除；失敗的話伺服器和本機都維持原狀，錯誤原文會傳到 UI。**這是目前整個信箱功能唯一一個會寫入伺服器狀態的操作**——見下方「這個設計不保證什麼」。

**搜尋**：使用者在 `MailView` 輸入自然語言 → `mail_search` → embed 查詢 → 本機 `mail_chunks` cosine+關鍵字比對 top-K，同信取最高分片段代表 → 分數過低 → AI 抽關鍵字/日期範圍 → IMAP SEARCH → 找到即快取入庫 → 回傳結果（寄件人/主旨/日期/既有 AI 摘要/命中片段）→ 前端列表；點選 → `MailDetailView` 顯示 `mail_messages.body_text` 全文與附件清單。

**回信**：使用者對某封信按「AI 回信」→ 可附加指示（如「婉拒」「確認時間」）→ AI 生成草稿存入 `mail_reply_drafts` → `ComposeReplyModal` 顯示可編輯 → 使用者按送出前走確認閘門（比照 `CommandPreview` 的 risk 確認 UI，寄出郵件視為必須手動確認的動作，絕不自動送出）→ 確認後 `mail_send_reply` 走 SMTP，正確帶 `In-Reply-To`/`References` header 維持信件串；可選擇夾帶本機檔案。

**廣告信清理**：使用者用自然語言觸發（例如在 `/ai` 或 Mail 面板輸入「幫我清掉廣告信」）→ `mail_list_promotional_candidates`：可選擇性用 AI 從查詢中抽取帳號/日期範圍篩選（重用 `search.rs` 的關鍵字/日期抽取邏輯），撈出對應帳號中 `is_promotional = 1` 的信 → 前端 `MailCleanupConfirm` 顯示候選清單（寄件人/主旨/日期/摘要），每筆預設勾選 → 使用者可取消勾選排除誤判 → 按確認 → `mail_delete_messages(message_ids)`：對每筆信執行 IMAP MOVE 到該帳號的 Trash 資料夾（伺服器不支援 MOVE 則退回 COPY + STORE `\Deleted` + EXPUNGE），成功後同步從 `mail_messages`/`mail_chunks`/`mail_attachments` 移除該筆快取。`is_promotional` 判斷本身在輪詢階段就已算好（與 `is_important` 同一次 AI 呼叫），觸發清理時不需要重新呼叫 AI 分類，只是把已有的分類結果拿出來給使用者確認。

這段描述的批次刪除機制底層要用的「移到 Trash」原語，已經在 Phase 1 以單封刪除的形式做出來了（見上方「刪除」一節、`mail/client.rs::move_to_trash`），第 6 期實作批次版本時直接重用即可，不必重新設計 MOVE/UIDPLUS 選擇或 Trash 資料夾解析這些細節。

## IPC 指令清單

**已實作（Phase 1）**：`mail_test_connection(input)`（存檔前驗證帳密，不寫入任何東西）／`mail_add_account` / `mail_remove_account` / `mail_list_accounts` / `mail_list_messages(account_id)` / `mail_mark_read(message_id)` / `mail_delete_message(message_id)`（單封，移到 Trash）/ `mail_count_unread`。

**規劃中（後續分期，尚未實作）**：`mail_search(query, account_id?)` / `mail_get_message(message_id)` / `mail_draft_reply(message_id, instructions?)` / `mail_send_reply(message_id, draft_text, attachments?)` / `mail_list_promotional_candidates(query?, account_id?)` / `mail_delete_messages(message_ids)`（複數批次版本；單封版本 `mail_delete_message` 已經在 Phase 1 做出來了）。

## 錯誤處理

| 情境 | 行為 |
|---|---|
| IMAP 連線失敗或中途斷線（帳密錯、網路斷、伺服器掛掉） | 該帳號的長連線任務以指數退避重連（5 秒起、每次翻倍，上限是設定的檢查間隔）；連線健康狀態的**轉換**（不是每次重試）發 `connection_failed`/`connection_restored` 事件，Mail 分頁顯示對應提示，純粹的重試過程只寫 log；不讓單一帳號故障影響其他帳號的任務 |
| 新增帳號時帳密或連線設定錯誤 | `mail_test_connection` 在寫入 config/keyring 之前先連線＋登入＋SELECT INBOX 驗證一次；失敗就直接把錯誤回給表單，什麼都不會被儲存，也不會啟動背景任務 |
| 伺服器 UIDVALIDITY 改變（信箱被重建過） | 該帳號本機快取的信件與同步游標整個歸零，從頭重新同步；一定會發 `removed` 事件讓 UI 更新 |
| 伺服器端刪除或搬移了信件（reconciliation） | 定期用 `UID SEARCH ALL` 比對本機快取與伺服器現況，本機多出來的信會被刪除並發 `removed` 事件；`UID SEARCH ALL` 本身失敗只會跳過那次比對，不影響已經同步進來的新信、也不會誤刪任何東西 |
| App 結束、或帳號被移除時任務還在忙 | 用 `watch::Sender<bool>` 通知任務優雅收尾（送 IDLE DONE + LOGOUT），給 5 秒；逾時才強制 `abort()`。多帳號時所有任務平行收尾，不會因為某一帳號卡住而拖慢整個 App 結束的時間 |
| 附件超過大小上限 | 只記 metadata（`skipped_too_large`），不下載內容、不解析——**規劃中，第 3 期，尚未實作**（Phase 1 完全不下載附件） |
| 附件類型不支援或解析失敗 | 只記 metadata（`unsupported_type` / `parse_failed`），不中斷該封信的摘要流程（用信件本文照常摘要）——**規劃中，第 3 期，尚未實作** |
| 搜尋時沒有可用 embedding provider | 搜尋功能停用（UI 顯示需先設定），不影響摘要/輪詢/回信——**規劃中，第 4 期，尚未實作** |
| 本機語意搜尋分數過低 | 自動退回 AI 關鍵字抽取 + IMAP SEARCH，找到後即時補建索引——**規劃中，第 4 期，尚未實作** |
| AI 分類（摘要／重要性／廣告信判斷）呼叫失敗 | log 後改用預設值（摘要留空、`is_important=false`、`is_promotional=false`）照樣插入信件，不會擋住這封信或整批同步；**沒有**重試機制或「已失敗」標記，摘要留空就是唯一看得到的訊號（原計畫寫的「標記失敗，使用者可手動重試」目前不成立） |
| 寄信失敗（SMTP 錯誤） | 草稿保留在 `mail_reply_drafts`，不視為已送出，UI 顯示錯誤並可重試——**規劃中，第 5 期，尚未實作**（Phase 1 完全沒有 SMTP 寄信能力） |
| 移除帳號 | 先優雅叫停該帳號的背景任務（等 LOGOUT，逾時才強制 abort）→ 從 config 刪除帳號設定 → 清除 keyring 憑證（best-effort，找不到也不算失敗）→ cascade 刪除本機快取。目前只有 `mail_messages`／`mail_poll_state` 兩張表在用；`mail_chunks`/`mail_attachments`/`mail_reply_drafts` 是規劃中的表，尚未建立，之後建了要記得把 cascade 補上 |
| 該信箱沒有 Trash 資料夾、或不支援 MOVE/UIDPLUS 任一種安全刪除原語 | 刪除直接回錯誤，不執行任何動作，UI 顯示原始錯誤訊息；絕不退回普通 `EXPUNGE`。單封刪除（`mail_delete_message`）已經是這樣實作；批次版本（`mail_delete_messages`）是規劃中的第 6 期功能，屆時會重用同一套規則 |
| 刪除請求逾時（該帳號的連線正忙於處理中的同步） | UI 顯示「可能還在進行中」而不是「失敗」，因為請求還留在該帳號的刪除 queue 裡，最終仍可能成功 |
| 清理批次中部分信件刪除失敗（例如已被其他用戶端搬走） | 該筆略過並在結果中標示失敗，其餘照常執行完，不整批中止——**規劃中，第 6 期，尚未實作**（單封刪除已經是「失敗就整個中止、回錯誤」，批次版本的部分失敗容錯還沒做） |

**安全邊界**：附件只做唯讀文字擷取，不執行巨集、不執行附件本身（第 3 期，尚未實作）；PDF/Office 解析器本身偶有安全漏洞，這是所有處理附件的信箱軟體都有的既有風險，本次不額外做沙箱隔離。寄出郵件（第 5 期，尚未實作）、刪除信件都一律需要使用者手動確認——單封刪除是列表上的兩次點擊（武裝→確認），廣告信清理走「候選清單勾選＋確認」（第 6 期，尚未實作），沒有任何路徑會自動送出或自動刪除。信件全文明碼存在本機 SQLite/檔案系統，與現有知識庫文件的儲存方式一致，不做額外加密。

## 測試

**Phase 1 實際落地的測試**（`src-tauri/tests/mail_classify.rs`、`mail_parse.rs`、`db_mail_integration.rs`，加上 `mail/poller.rs`、`mail/client.rs` 檔案內的 `#[cfg(test)]` 單元測試）：AI 分類 prompt 與 JSON 解析用假的 `AiProvider` mock，不是 wiremock；沒有真的起一個 IMAP/SMTP server 來測，`plan_fetch_batches`/`collect_batches`/`should_reconcile`/`uid_validity_changed`/`resolve_trash_mailbox`/`choose_delete_strategy`/`next_failure_count`/`health_report` 這些純邏輯用假資料源（`FakeSource`）跟純函式單元測試涵蓋；DB 層用真的 SQLite（記憶體或暫存檔）測 schema 升級、cascade 刪除、reconciliation 的刪除集合計算。原計畫寫的「wiremock 模擬 IMAP/SMTP」沒有發生——這條技術選擇沒有被驗證過是否可行。

**規劃中（隨對應分期一起做，尚未實作）**：
- 附件大小超限/類型不支援時只記 metadata，不阻斷信件摘要（第 3 期）
- 搜尋：本機 cosine+關鍵字排序正確性；分數過低時觸發 IMAP SEARCH fallback（第 4 期）
- 寄信失敗時草稿不被標記為已送出（第 5 期）
- 廣告信清理：候選清單只來自 `is_promotional = 1` 的信；沒有 Trash 資料夾時整批回錯誤且不刪除任何信；批次中單筆刪除失敗不影響其他筆（第 6 期；單封版本的「沒有 Trash 就整批回錯誤、絕不退回 EXPUNGE」已經在 Phase 1 用純函式單元測試涵蓋了，見 `client.rs` 的 `resolve_trash_mailbox`/`choose_delete_strategy` 測試）

**前端 Phase 1 實際落地的測試**（Vitest + React Testing Library，`MailView.test.tsx`、`MailAccountsPage.test.tsx`、`useMailSync.test.ts`）：`MailView` 的連線失敗 banner 顯示/清除、單封刪除的兩次點擊確認流程；`MailAccountsPage` 存檔前必須先 `mail_test_connection` 成功才會呼叫 `mail_add_account`；`useMailSync` 的未讀數重新整理與 `important` 事件觸發通知的時機。

**規劃中（隨對應分期一起做，尚未實作）**：
- `ComposeReplyModal` 送出前必須經過確認閘門，不能一按就送（第 5 期）
- 搜尋結果點選後 `MailDetailView` 正確顯示全文與附件（第 4 期）
- `MailCleanupConfirm` 預設全選、取消勾選後該筆不會出現在送出的 `message_ids` 裡、沒有勾選任何一筆時確認按鈕為 disabled（第 6 期）

依 TDD，每個測試都要先確認會紅再寫實作。

## 已考慮並否決的方案

**輪詢由前端 `setInterval` 驅動，後端只提供單次 fetch/summarize command。** 實作較簡單，但多帳號時前端要自己管多組 timer 與並行 invoke，且與現有「後端擁有狀態、前端訂閱事件」的架構風格（`pty/`、`ai/`）不一致，可靠度也較差。否決。

**搜尋只做 AI 關鍵字抽取 + 既有 FTS，不做向量語意搜尋。** 實作更輕、不需要使用者設定 embedding provider，但對語意/情緒類查詢召回率較差；使用者明確要求語意搜尋能力，且知識庫既有的向量管線可直接重用，成本不高。否決。

**附件只顯示 metadata，不解析內容。** 最簡單、無安全疑慮，但使用者的核心情境（報價單）很常見是 PDF 附件而非信件本文，不解析會讓搜尋直接找不到，違背功能初衷。否決。

**App 關閉後仍背景常駐輪詢。** 體驗更即時，但需要 tray icon 或 OS 服務等跨三平台的常駐機制，複雜度與維護成本大幅上升，且使用者已確認「只在 App 開啟時輪詢」即可。不在本次範圍，列為未來可能的延伸。

**AI 判定為廣告信就直接刪除，不經使用者確認。** 少一道操作、體驗更順，但刪信是難以復原的動作，AI 分類一定會有誤判，沒有確認步驟等於把誤判的代價完全轉嫁給使用者且無法補救。否決，一律先列候選清單讓使用者勾選確認。

**清理時對整個信箱歷史（含尚未輪詢過的舊信）逐封掃描分類。** 涵蓋範圍更完整，但對大信箱會是大量 AI 呼叫、耗時且成本高，且使用者已確認「只處理已輪詢過的信」即可。不在本次範圍。

## 這個設計不保證什麼

**AITerm 現在會寫入伺服器狀態，這不再是原本「完全唯讀」的保證。** 最初的設計是「AITerm 完全不動伺服器狀態」（收信用 `BODY.PEEK`，已讀狀態只記本機、不碰 `\Seen`）。Phase 1 出現了第一個、目前也是唯一一個例外：使用者在 Mail 列表對單封信按下刪除（兩次點擊確認）時，AITerm 會對伺服器發 `UID MOVE`（或退回 `UID COPY` + `UID STORE \Deleted` + `UID EXPUNGE`）把信搬進 Trash。這永遠是使用者手動、明確觸發的動作，沒有任何自動路徑會做這件事——但它確實是一次寫入，讀這份文件的人不該再假設「AITerm 只讀不寫」對整個功能通用。第 6 期的批次廣告信清理會重用同一個原語，屆時寫入路徑會更多，但性質不變：一律要使用者確認。

**伺服器端的刪除/搬移不是即時反映到本機的。** 偵測靠定期的 `UID SEARCH ALL` 比對（連線剛建立、存活逾時、比對到期時一定做；單純收到 IDLE 推播通知不會馬上做，除非距離上次比對已經超過一個檢查間隔），所以在另一台裝置上刪掉的信，本機列表最慢要等到下一次比對週期才會消失，不是秒級同步。

**連線失敗的 UI 提示只在「狀態轉換」時出現一次，不代表背後停止重試。** 帳號斷線後會用指數退避持續重連，畫面上的失敗提示只在剛斷線那一刻跳出來一次，中間每一次重試都不會再打擾使用者、也不會有 OS 通知；如果遲遲沒看到「已恢復」，代表重連還在進行中，不是卡住不動又完全沒有跡象——但也沒有任何地方顯示「第幾次重試」或「下次重試還要多久」。

**語意搜尋的召回範圍受限於「已輪詢過的信」。** 伺服器上還沒被輪詢抓過的舊信，語意向量並不存在，只能靠關鍵字 IMAP SEARCH 退回機制找到，且找到後才會補建索引——第一次搜到某封舊信時，語意比對其實還沒發生。（這條是第 4 期的設計限制，該功能本身還沒實作。）

**「重要信件」與「廣告信」的判斷完全交給 AI，沒有使用者可調的規則或門檻。** 這代表判斷標準不可預測、也無法保證跟使用者的主觀認定一致；如果誤判過多或過少，本次設計沒有提供使用者側的調整機制（只能靠之後的疊代改 prompt，或靠清理流程裡的手動勾選排除來補救單次誤判）。目前程式碼裡有一條額外的保險——分類結果如果 `is_promotional` 為真，`is_important` 會被強制覆寫為假，不完全信任模型自己遵守「廣告信不算重要」這條 prompt 規則——但這只縮小了「廣告信被誤判成重要」這一種誤判，其餘方向的誤判（例如真正重要的信被判成不重要）仍然完全沒有調整機制。

**廣告信清理涵蓋不到尚未輪詢過的舊信。** 跟語意搜尋一樣，`is_promotional` 只在輪詢當下算過，本次範圍明確不做「回頭掃描整個信箱歷史」，所以清理功能天生只對「AITerm 開始監控之後收到的信」有效。（`is_promotional` 欄位跟分類邏輯本身在第 1 期已經跑在每封信上；只是候選清單/批次確認/批次刪除這個 UI 流程還沒做，是第 6 期的範圍。）

**附件文字擷取的品質取決於檔案本身的結構。** 掃描版 PDF（純圖片、沒有文字層）擷取不到任何文字，會被視同「沒有可用內容」，摘要與搜尋都無法涵蓋——這不是解析失敗，而是這次範圍明確不含 OCR 的直接後果。（這條也是第 3 期的設計限制，附件下載/擷取本身還沒實作。）
