# 問題收集 Server 部署指南

`aiterm-intake-server` 是常駐的問題收集 server：收 LINE 訊息、保存共用的問題清單，讓團隊每個人的 AITerm 連上來看同一份清單與處理狀態。設計見 `docs/superpowers/specs/2026-11-23-intake-server-design.md`。

## 一、準備與啟動

```bash
# 1. 建資料夾、金鑰、自簽憑證與資料庫（可重複執行，不會覆蓋既有的金鑰與憑證）
aiterm-intake-server init --san intake.example.com --san 192.168.1.20

# 2. 建專案（可多個）
aiterm-intake-server project add 範例科技

# 3. 產生第一個管理員的邀請碼（15 分鐘內有效、只能用一次）
aiterm-intake-server pair --name 管理員 --role admin --address intake.example.com:8443

# 4. 啟動（預設 API 在 0.0.0.0:8443；LINE webhook 只收本機）
aiterm-intake-server run
```

資料夾預設是 `~/.aiterm-intake`，可用 `--data-dir` 改。裡面有 `intake.db`（問題清單與設定）、`server.db`（成員、邀請、稽核、專案）、`secrets.key` 與 `secrets.json`（加密後的祕密）、`tls/`（憑證）。**備份整個資料夾**；`secrets.key` 遺失就解不開 `secrets.json`。

## 二、成員連線

成員在 AITerm 的「設定 → 共用 server」輸入 server 位址與邀請碼（問題收集、工作看板、Agent 協作都走同一條連線）。每位成員一組邀請碼：

```bash
aiterm-intake-server pair --name 小明 --role member
aiterm-intake-server members list
aiterm-intake-server members revoke 小明     # 已連線的 AITerm 會在幾秒內被中斷
```

- **member**：看與處理題目（核准、駁回、刪除、改草稿、回覆、領取處理）。
- **admin**：另可改設定、白名單、專案、祕密與 AI 設定。
- 連到 server 期間，那台電腦自己的收訊與派送會停止（避免同一封信被收兩次）；中斷連線才恢復。

### 記錄多組 server

同一個 AITerm 可以記錄多組 server（例如公司與自己的測試機），**一次只連一組**。在「設定 → 共用 server」的「已記錄的 server」：

- **新增**：按「＋ 新增 server」，輸入位址與邀請碼；連上後記在清單裡，並從目前這組切過去。同一個位址再配對一次，會更新原本那筆（名稱、角色、憑證指紋），不會變成兩筆。
- **切換**：按其他那筆的「連線」，**不用再輸入邀請碼**。切換前會先確認對方連不連得上：連不上會先問「仍要切換過去嗎？」，確定才切，切過去後是離線狀態，對方恢復會自動更新；憑證改變、版本不相容這類問題不給硬切，只顯示原因。
- **領來的工作還在處理時不能切換**：從 server 領來、還在本機處理的題目或看板卡片，結果要回到原來那台，所以要先完成或放回，才能換 server。
- **中斷連線**只是切回本機，**記錄與登入資訊都保留**，之後按「連線」就能連回去；本機的收訊與派送在中斷時恢復。
- **刪除**：先確認，連同這台電腦鑰匙圈裡那組的登入資訊一起清掉；之後要連回去得重新用邀請碼。server 端的成員紀錄不受影響，要撤銷請管理員用 `members revoke`。
- 每一組的登入資訊各存在系統鑰匙圈的一個項目（`intake_remote_token:<記錄 id>`）。從舊版升級時，原本唯一的那組連線會自動變成清單的第一筆並維持連線，不需要重新配對；舊的鑰匙圈項目在第一次成功讀到時搬過去。
- 「在我的電腦處理」的本機專案對應、各專案的參考資料夾都是以 **server 專案** 為單位記錄，不同 server 的專案不會互相覆蓋。
- 目前不支援同時連多組 server。

## 領取處理

題目要先是「待派送」才能被領（內部題目自動進入；外部題目要先核准）。

- **手動**：在 AITerm 清單選一題，按「我來處理」，會派給你「收集中」且閒置的終端機分頁。
- **自動**：在終端機工具列按「啟動問題收集」，選 server 的專案；這個分頁閒置、Claude Code 在跑時，會自動向 server 領下一題。
- 領取是 server 上的原子動作；清單上所有人看到「處理中：誰」。租約 15 分鐘，處理中每 2 分鐘自動續約；關機或斷線就自動放回，別人可以領。
- 做完的回答交回 server 變成**草稿**，由人看過再用 LINE 送出，不會自動寄出。分頁關閉或 Claude Code 結束會回報失敗並記原因；停止收集會把題目放回去。
- 領取者與管理員可以放回；只有領取者能交結果。
- **做完後的驗證**：「要改程式」的題目（server 沒標種類時一律當成要改程式；標了是提問、或這個管道設成「只診斷」的不驗證），派工前會在你終端機的工作資料夾記下 git HEAD，做完後用專案設定的「驗證指令」在**你的工作資料夾**執行，報告（改了哪些檔案、測試過不過）附在草稿最後。驗證沒過**不會擋住**交回，但問題清單會標示「驗證未通過」，提醒審稿的人。
- **參考資料夾**：server 設定裡的參考資料夾是 server 上的路徑，不會帶給你的 Claude Code。要讓你自己的 Claude Code 讀到資料，請在專案設定頁的「我電腦上的參考資料夾」填這台電腦上的完整路徑（分「共用」與「僅內部」兩組；客戶的外部題目只會用到共用那組）。這份設定只存在你的電腦，任何成員都能設，不會傳到 server。
- 第一版不經看板與 Agent 協作。

## Server 自動處理題目

Server 可以自己用 Claude Code 處理題目（不必等成員領取）。結果跟成員處理一樣是**草稿**，由人看過才送出。

1. **在 server 主機安裝並登入 Claude Code**：在主機上安裝 `claude`，執行一次 `claude login`。AITerm server 不存也不讀這份登入。連上 AITerm 後，專案設定頁的「Server 自動處理（Claude Code）」卡片會顯示 Claude Code 能不能用；路徑不在 PATH 上時在這裡填完整路徑。
2. **專案設定工作資料夾**：server 專案的 `work_dir` 要指到 server 上的程式碼資料夾（可改檔案的題目要求它是有提交紀錄的 git 專案）。
3. **開啟「由 server 自動處理這個專案的題目」**（專案設定頁，預設關）。已核准、待派送的題目會被 server 領取處理；與成員手動領取並存，誰先領到誰處理。Claude Code 不能用時 server 不會領任何題目，留給成員。

**兩種模式**
- **唯讀（預設）**：Claude Code 只拿到讀檔與搜尋工具，明確禁用改檔案、執行指令與網路。以下一律唯讀：外部客戶的題目、管道設成「只診斷」、提問類題目、沒標種類又無法分類的題目。參考資料夾（server 上的路徑）會開放給它讀。
- **可改檔案（需管理員核准）**：只對內部、管道設成「可改檔案」、種類是「要改程式」的題目。Claude Code 在隔離的 git worktree 與專屬分支（`intake/…`）裡改，除了讀寫檔案，只多開放專案設定的「驗證指令」這一個指令；做完 commit、跑驗證，題目詳情會顯示「server 自動處理的改動」與驗證報告。**永不自動合併**：管理員按「核准並合併」才會以 `--no-ff` 合併進專案目前的分支（衝突時會還原、什麼都不改並回報，可以處理後再試）；按「駁回」就刪除分支。核准之前專案本身不會被改。

**Windows**：Claude Code 的執行檔要填完整路徑（例如 `C:\\Users\\…\\claude.exe`），server 不會自己找 `.cmd` 外殼。

**限制與安全**：同時處理題數、每題逾時（上限 800 秒）、回合上限都可在卡片調整；逾時會終止 Claude Code；傳給它的環境變數只有必要的幾個；回覆與驗證報告先去敏。沒有串流顯示處理過程，只看得到最終結果。已用真實 Claude Code 驗證：唯讀問答讀得到專案檔案；改檔案題在隔離分支改完、驗證通過，核准後合併。

## 共用工作看板（第一階段）

連到 server 後，看板專案清單多了「🌐 專案名」（server 上的專案）。大家看到同一份卡片：建立、編輯、移動欄位、標籤、附件、封存、刪除都走 server，其他人的畫面即時更新。

- **領取**：待派送欄的卡片按「我來處理」。先在看板上方設定「我電腦上的專案」（影子卡片放在哪個本機看板專案、在哪個資料夾執行，只存在你的電腦）。領取是 server 上的原子動作，同一張只有一個人拿得到；領到後在你的本機看板建一張標示「🌐 server」的影子卡片，走原本的派工流程（你自己的終端機、worktree、對話紀錄都留在你的電腦），做完結果回寫 server。處理中每 2 分鐘自動續約，關機或斷線 15 分鐘後 server 自動放回待派送；影子卡片被刪掉或拖回計畫中也會放回。
- **衝突**：多人同時改同一張卡，後存的會被告知「這張卡片已被別人修改」，重新載入後再改，不會靜默覆蓋別人的修改。
- **權限**：成員可以建立、編輯、移動、領取；刪除限卡片建立者或管理員；整欄封存限管理員。處理中的卡片不能編輯、刪除或封存。
- **Agent 協作**見下一節；**目前不含**跨電腦接手同一張卡的本機狀態。

## 共用 Agent 協作與 server 執行（第二階段）

選了「🌐 專案名」之後，Agent 協作畫布（Coder、審核者、協調者、定時器、專案簡介、記事本）也是共用的：誰改了，大家看到同一份。Agent 的名稱、角色、提示詞前綴與**模型**存在 server（技能、資料庫、系統連線與程式庫資料夾是各台電腦的環境，不共用；建立 Agent 時填的程式庫只在你的電腦）。

**由 server 執行**（畫布頂列的「☁」開關，管理員才能開；專案要先設定工作資料夾 `work_dir`，server 主機要裝好並登入 Claude Code）：

- 指派給 Coder 的卡由 server 自己做：每位 Coder 有專屬的隔離 worktree 與分支（`agent/<id 前 12 碼>`），一位 Coder 同一時間只做一張卡；開了以後成員不能手動領有指派 Agent 的卡。
- 做完由審核者**唯讀**審查（看被審那版的 diff）；不通過就把理由退回 Coder 修正，最多 2 輪，用完就停下來等人。專案設了「驗證指令」時，server 會在 Coder 完成後實際執行它，結果一併交給審核者。
- 指派給「產出規格」的 Coder 的卡是規格卡：審查通過後交給協調者拆成實作卡（沿用桌面版的任務、相依、介面約定格式），分給最閒的 Coder；有相依的卡等前置通過審查才開工，並先把前置通過審查的版本合併進自己的 worktree。
- **永不自動合併**：所有模組都通過審查後，頂列下方出現「核准合併」橫幅，管理員按兩下（第一下只問確認）才會把各 Coder 的分支合併進專案資料夾；還有模組在做或沒過審時不能核准。
- server 重啟（更新、當機、電腦休眠後被拉起）時，上一個行程正在做的卡片與題目會**立刻放回待派送**，不必等租約逾期；成員自己領的不受影響。
- **Coder 的卡會接著做，不重頭開始。** Coder 做卡片用固定的 Claude Code 對話 id；被打斷（server 重啟、額度用完）後，下一次用 `--resume` 接回同一段對話，並提示它先看工作資料夾裡留下的進度、不要重做已完成的部分。第一次開工的起始版本也一起記下，審查的 diff 才不會漏掉上次已 commit 的部分。對話檔不見了（被刪、換了機器）就改開一段新的，不算失敗。卡片做完（成功或失敗）才清掉對話記錄，**並刪掉 Claude Code 留在 `~/.claude/projects/` 底下的那份對話檔**，不會在你平常的對話清單裡留下一堆記錄。審查、拆解這類唯讀的短任務不留對話。
- 專案沒設「驗證指令」時，Coder 在無頭環境沒有執行指令的權限（不能跑測試、編譯）；server 會在提示詞裡明講，Coder 只能讀程式碼推演並註明「未實際執行測試」。要有實測請設驗證指令。
- 協調者在 server 上只負責拆解規格；它的角色說明若寫「合併分支、跑測試」（桌面版預設），拆解提示詞會明確要它忽略，合併一律由管理員核准。
- 額度或速率限制不算失敗：卡片放回待派送，等到重置時間（錯誤訊息有時間就用它，沒有就等 20 分鐘）自動續跑，最多等 12 次。

**定時器**：每天／每週／間隔建一輪卡片，**只由 server 觸發**，多人同時連線也只會建一次。時間點預設是 server 主機的本地時間；對話框的「時區」欄位可以改成任何 IANA 時區（例如 `Asia/Taipei`），新增時預設帶你這台電腦的時區，「每天 09:00」就是你的 09:00，不用換算 server 在哪個時區（夏令時間會自動處理）。上一輪還沒結案就跳過；一輪成功後把完成的卡歸檔；連續 3 輪失敗或目標角色被刪就自動停用並寫明原因。專案開了「由 server 執行」時，定時的卡由 server 做完並審查，通過才算一輪成功；沒開就留給成員領取。建立、修改、停用、立即執行限管理員。

**尚未支援（在 server 上）**：驗收者（需要實際操作畫面）、Coder 中途詢問人類（`claude -p` 是非互動的）、編制切換、技能（資料庫、自訂、系統連線）、Agent 之間的即時對話。桌面版本機專案的這些功能不受影響。

## 三、憑證

- **內網／VPN（預設）**：`init` 產生自簽憑證。AITerm 在配對時釘住它的指紋，之後只信這張；憑證改變時 AITerm 會警告並拒絕連線，必須重新配對。
- **公開網路**：用正式憑證：`aiterm-intake-server run --tls-cert fullchain.pem --tls-key privkey.pem`。AITerm 配對後會先試著用一般的根憑證驗證，通過就不釘指紋，憑證換新不會誤報。（這個模式目前只有自動測試的部分涵蓋，還沒有用真實的正式憑證驗收。）

## 四、LINE

1. 在 LINE Developers 的 Messaging API 取得 Channel secret 與 Channel access token，由管理員在 AITerm 的 LINE 分頁填入（只寫入，不會被讀出）。
2. 啟用 LINE、設定埠（預設 8321）後，server 在 `--channel-bind`（預設 127.0.0.1）的該埠接 `/line/webhook`。LINE 要求 webhook 是公開的 HTTPS，請用反向代理（Caddy、nginx）或 Tunnel（ngrok、Cloudflare Tunnel）把公開網址轉到這個埠；若代理在另一台機器，用 `--channel-bind 0.0.0.0` 並自行限制來源。
3. 把 LINE 的 webhook URL 設為 `https://<公開網址>/line/webhook`，並打開「Use webhook」。

## 五、AI（用於題目分類與群組「是不是新問題」）

```bash
echo "$API_KEY" | aiterm-intake-server ai --provider openai --model <模型> 
# provider：openai、anthropic、openai_compat（再加 --base-url）、ollama（不需要金鑰）
```

也可以由管理員在 AITerm 設定：連上 server 後，到問題收集的設定頁最上方的「Server AI」卡片，選供應商、填模型（與位址、金鑰）按「儲存到 server」；金鑰只寫入、不會被讀回，留空代表不變。訂閱帳號（Claude、ChatGPT、Gemini）見下。沒設定時，LINE 群組每段安靜的訊息都會被當成新問題，建議設定。

**用 Claude 訂閱登入（不用 API 金鑰）**：連上 server 後，在「Server AI」卡片的供應商選「Claude 訂閱」，按「為 server 登入」。AITerm 會在你的電腦開瀏覽器走一次授權；把瀏覽器顯示的授權碼貼回來按「完成登入」，憑證就只交給 server（**不會**存進你這台電腦的鑰匙圈，你自己電腦上的 Claude 登入是另一組，互不影響）。之後由 server 自己續期與呼叫；續期憑證是一次性的，server 會讓同時進來的請求排隊共用一次續期。憑證失效時卡片會顯示「登入已失效」，重新登入即可。

- 為什麼要「另外登入一次」而不是複製你電腦上的登入：Claude 的續期憑證用一次就失效，兩邊共用同一組會互相把對方登出。
- 憑證加密存在 server 的祕密庫，不會被讀回；只有管理員能登入與登出。登出會刪除 server 上的憑證，但供應商那端的授權要到帳號頁面自行撤銷。
- 訂閱帳號由整個團隊的請求共用，請自行確認供應商的使用條款。
- **ChatGPT 訂閱、Gemini 訂閱**：供應商選「ChatGPT 訂閱」或「Gemini 訂閱」，按「為 server 登入」。這兩個不用貼授權碼：AITerm 在你的電腦開瀏覽器，授權完瀏覽器會導回你電腦的本機網址，由 AITerm 自己收下（最久等 2 分鐘）。ChatGPT 固定用 1455 埠，被占用（例如正在登入 Codex）時會明講；Gemini 登入後會自動開通 Cloud Code 專案，失敗就整個登入失敗、不留半套。登入完成後要選一個模型再按「儲存到 server」。目前只有自動化測試驗證過，尚未用真實帳號驗證。
- 一次只能有一個作用中的訂閱：換一家登入會清掉上一家的憑證與模型名稱。
- 卡片另有「載入模型清單」、「測試連線」（用還沒儲存的值也能測）與「最近一次呼叫」的成功或失敗。

## 網頁表單與寄信帳號

網頁表單讓白名單內的人用電子郵件驗證碼登入後提問，所以 server 需要自己的寄信帳號（SMTP）來寄驗證碼與回覆：

```bash
echo "$SMTP_PASSWORD" | aiterm-intake-server mail --smtp-host smtp.example.com --smtp-port 587 --username bot@example.com [--from help@example.com]
```

在 AITerm 連上 server 後，也可以不用命令列：到問題收集的設定頁、「郵件」分頁，選一個 AITerm 已有的郵件帳號按「套用到 server」，就會把它的 SMTP 設定與密碼（從鑰匙圈讀出）寫到 server，只寫入、不會被讀回，且只有管理員做得到。設定後 `status` 會顯示寄信帳號；也能用 API（`GET/PUT /v1/mail`，密碼走 `PUT /v1/secrets/smtp_password`，不會被讀回）。

在 AITerm 連上 server 後，到設定頁把專案的「網頁表單」設成「僅本機」或「區網」，server 就會在設定的埠（預設 8320）開出表單，網址是 `http://<server 位址>:8320/`，每個專案有自己的 `/p/<代碼>` 頁。任一專案設成區網時綁 `0.0.0.0`，否則只收本機；要放到公開網路請放在反向代理（HTTPS）後面。表單的回覆以信件寄回提問者。

## Email

Server 可以用 IMAP 收信、用 SMTP 回覆，規則與桌面版相同（別名或主旨標籤、白名單、內部／外部）。

- 設定：在 AITerm 連上 server 後，設定頁「郵件」分頁選一個已有的郵件帳號按「套用到 server」，會把 IMAP 與 SMTP 設定與密碼一起寫到 server；或用命令列：`echo "$密碼" | aiterm-intake-server mail --smtp-host … --imap-host … --username …`。
- 每 60 秒讀一次信箱，**唯讀**（只讀取，不改已讀、不搬、不刪）。有任一專案啟用問題收集就會收；信屬於哪個專案，依各專案的別名或主旨標籤決定，整個 server 共用這一個信箱。
- **第一次只記下目前信箱的位置，不收既有的舊信**；之後只收新進來的。信箱換了身分（UIDVALIDITY 改變）時同樣重新記位置，不補收。一次輪詢最多收 100 封，剩下的下一輪接著收。
- 回覆用信件寄出，串在原信的對話裡並回覆全部收件人（去掉收信帳號與別名自己）。
- 只支援隱含 TLS 的 IMAP（通常是 993），不支援 143 的 STARTTLS。`status` 會顯示是否收信、最近成功時間與錯誤。
- 已用真實信箱驗證過一輪（寄出標有 `[AITerm]` 的信，約一分鐘內收進清單）。

## Telegram

Server 可以用 Telegram 機器人收訊息並回覆，規則與桌面版相同：主人的聊天室用 `/ask 問題` 直接提問（可在 `/ask` 後面接專案名稱），其他聊天室要先把 `tg:<聊天室 id>` 加進專案白名單。

- 設定：連上 server 後，設定頁的「Telegram」分頁最上方有「Server Telegram 機器人」卡片，填機器人金鑰（與主人聊天室 id，選填）按「儲存機器人設定」，只有管理員做得到；金鑰只寫入、不會被讀回，留空＝不變。也可用命令列：`echo "$機器人金鑰" | aiterm-intake-server telegram --owner-chat-id 123456`。
- 用長輪詢（getUpdates）收，不需要公開網址。設定好、且至少一個專案啟用 Telegram 就會開始收；卡片會顯示「收訊中」或錯誤。
- **同一個機器人同一時間只能有一個程式在輪詢**（否則 Telegram 回 409，卡片會顯示這個錯誤）。給 server 用的機器人，不要同時設在任何一台桌面版 AITerm 的 Telegram 設定裡；建議另外開一個機器人給 server。
- 回覆用同一個機器人，以「回覆原訊息」的方式送回該聊天室，過長會切成多則。
- 目前只有自動測試涵蓋（假的 Telegram 伺服器：收訊、白名單、主人指令、回覆、409 衝突），還沒用真實機器人驗收。

## 六、systemd 範例（Linux）

```ini
[Unit]
Description=AITerm Intake Server
After=network-online.target

[Service]
ExecStart=/usr/local/bin/aiterm-intake-server --data-dir /var/lib/aiterm-intake run --bind 0.0.0.0:8443
Restart=on-failure
User=aiterm
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
```

## 六之二、macOS 常駐（launchd）

把下面存成 `~/Library/LaunchAgents/com.aiterm.intake-server.plist`（路徑換成你的），登入後自動啟動、被結束會自動重啟：

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.aiterm.intake-server</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/aiterm-intake-server</string>
    <string>--data-dir</string><string>/Users/你/.aiterm-intake</string>
    <string>run</string>
    <string>--bind</string><string>0.0.0.0:8443</string>
  </array>
  <key>EnvironmentVariables</key><dict><key>RUST_LOG</key><string>info</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/Users/你/.aiterm-intake/server.log</string>
  <key>StandardErrorPath</key><string>/Users/你/.aiterm-intake/server.log</string>
</dict>
</plist>
```

```bash
launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/com.aiterm.intake-server.plist   # 啟動並載入
launchctl bootout   gui/$(id -u)/com.aiterm.intake-server                                 # 停止並卸載
```

- 這是**登入後**才會跑的 LaunchAgent；沒有人登入時 server 不會啟動。要開機就跑（不需登入）得改用 `/Library/LaunchDaemons` 並用 root 載入，這一種沒有測過。
- 已在 Apple Silicon 的 macOS 上用發佈版實測：載入後能回應 `/v1/hello`；結束行程後約 10 秒內由 launchd 自動重啟並恢復服務；卸載後行程確實停止。
- 第一次執行下載來的檔案若被 Gatekeeper 擋下，用 `xattr -d com.apple.quarantine <檔案>` 解除（用 `gh` 或 `curl` 下載的檔案通常不會被標記）。
- 要讓區網的其他人連進來，系統設定的防火牆需要允許這個程式接收連入連線。

## 六之三、Windows 常駐（工作排程器）

`aiterm-intake-server.exe` 不是 Windows 服務程式，不能直接用 `sc create`。建議用工作排程器在開機時啟動，失敗自動重試（以系統管理員的 PowerShell 執行）：

```powershell
$a = New-ScheduledTaskAction -Execute 'C:\aiterm-intake\aiterm-intake-server.exe' `
     -Argument '--data-dir C:\ProgramData\aiterm-intake run --bind 0.0.0.0:8443'
$t = New-ScheduledTaskTrigger -AtStartup
$s = New-ScheduledTaskSettingsSet -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1) `
     -ExecutionTimeLimit ([TimeSpan]::Zero) -StartWhenAvailable
Register-ScheduledTask -TaskName 'AITerm Intake Server' -Action $a -Trigger $t -Settings $s -User 'SYSTEM' -RunLevel Highest
Start-ScheduledTask -TaskName 'AITerm Intake Server'

# 讓區網連得進來
New-NetFirewallRule -DisplayName 'AITerm Intake Server' -Direction Inbound -Protocol TCP -LocalPort 8443 -Action Allow
```

- `init`、`project add`、`pair` 要用**同一個資料夾**（`--data-dir C:\ProgramData\aiterm-intake`）事先執行好，再啟動排程。
- 停止與移除：`Stop-ScheduledTask` 與 `Unregister-ScheduledTask -TaskName 'AITerm Intake Server' -Confirm:$false`。
- **這一段沒有在 Windows 上實測過**（只確認了發佈的 Windows 版有建置成功並在建置時執行過一次），第一次使用請先手動執行 `run` 確認程式能啟動。

## 七、目前的限制

- Server 預設**不會自動處理題目**（要在專案設定開啟，見「Server 自動處理題目」）。沒開時處理靠成員領取：在 AITerm 清單按「我來處理」，或讓「收集中」的終端機分頁閒置時自動領；題目在領取者自己的電腦用 Claude Code 處理，做完交回 server 變成草稿，由人看過再送出。
- 支援 **LINE**、**網頁表單**、**Email** 與 **Telegram** 收訊與回覆。
- 看板與 Agent 協作可以在 server 上共用與執行（見「共用 Agent 協作與 server 執行」）；驗收者、技能等尚未支援。
- 所有成員看得到所有專案。

## 八、發佈（給維護者）

Server 版用 `intake-server-v<版本>` 這種 tag 發佈（例如 `intake-server-v0.1.0`），發佈 workflow 只會建立**草稿** release，檔案齊全後由人公開。

**公開時一定要用指令，不要在 GitHub 網頁按 Publish：**

```bash
gh release edit intake-server-v<版本> --draft=false --latest=false
```

網頁的 Publish 會自動把這則 release 標成整個 repo 的「Latest」。桌面版的自動更新讀的是 `releases/latest/download/latest.json`，Latest 一旦被 Server 版搶走，桌面版使用者的更新檢查就會 404（v0.1.0 公開時發生過，改回 `gh release edit v<桌面版> --latest` 後恢復，但 GitHub 的快取會讓原網址暫時還是 404）。

公開後確認桌面版沒被影響：

```bash
gh release list --limit 3 --json tagName,isLatest   # 桌面版 v1.x 必須是 isLatest=true
```
