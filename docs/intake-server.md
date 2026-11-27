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

**尚未支援（在 server 上）**：Coder 中途詢問人類（`claude -p` 是非互動的）、編制切換、技能（資料庫、自訂、系統連線）、Agent 之間的即時對話。桌面版本機專案的這些功能不受影響。

## Server 驗收者（共用 Agent 協作的第三階段）

全部模組審查通過後、出現「合併待核准」**之前**，server 先實際驗收一次：確認各工程師的成果合併起來真的能用。設計見 `docs/superpowers/specs/2026-11-26-server-verifier-design.md`。

**啟用條件**（都要有，否則流程和以前完全一樣）：
- 畫布上有一位「驗收者」（共用專案每個最多一位，可在角色設定指定較便宜的模型）。
- 專案設定了工作資料夾（`work_dir`，要是有提交紀錄的 git 專案）並開了「由 server 執行」。
- 管理員在畫布頂列按「🔍 由 server 驗收」開啟（成員看得到狀態、不能改）。
- server 那台電腦要有 Claude Code（已登入）。網頁類專案要有 Node.js（`npx`）；有 Google Chrome 最好，沒有會自動改用 Playwright 的 Chrome for Testing（第一次要下載約 100MB，不需要管理員權限）。沒有 Node.js 時，需要瀏覽器的條件會被標成「需人工確認」，不會當成通過。

**流程**：
1. 全部模組通過 → 建一張**驗收卡**（不再直接建合併卡）。
2. server 開一個**拋棄式的整合工作資料夾**（專案目前的版本），依序把各工程師的分支併進去，**不碰專案資料夾**。分支之間合併衝突時驗收直接失敗，原因寫「合併衝突」（核准合併時也會衝突，所以要先處理）。
3. 驗收者（Claude Code）在那裡實際建置、啟動伺服器、用無頭瀏覽器點擊與截圖、呼叫端點、跑測試，把結果用 Bash 寫成 `verify.json`（每個條件 `pass`／`fail`／`manual`，附重現步驟、預期與實際、證據）。**驗收者沒有編輯檔案的工具**，不會改你的程式。
4. 結果：
   - **通過** → 才出現「合併待核准」，內文寫明已通過驗收（有幾項需要你確認）。
   - **退回** → 依失敗項對每個有對應模組的群組開一張修正卡給原工程師，修好後重新審查、**重新驗收**；最多自動修正 2 輪，第 3 次失敗就停下來等管理員。
   - **需要你決定** → 失敗項找不到屬於哪個模組、驗收卡自己失敗（合併衝突、沒寫 `verify.json`、報告沒附證據）、或**沒有任何條件被實際驗證**（全部標成需人工確認，例如伺服器被禁止清單擋下、瀏覽器打不開）。管理員看報告後可以「放行」（視為通過）或「重新驗收」。
5. 驗收結束（不論成敗）：關掉驗收者啟動的伺服器與瀏覽器、刪掉整合工作資料夾與 Claude Code 留下的對話檔。驗收中 server 被強制結束時，重啟會把孤兒的 Claude、Playwright 與佔著驗收用埠的伺服器一併結束，然後**重新驗收**（不接續對話：整合工作資料夾是拋棄式的，伺服器與瀏覽器的狀態都已消失）。

**驗收報告**：點驗收者卡片開「驗收報告」，每個條件的結果、重現步驟、預期與實際、截圖縮圖（點開放大）、被禁止清單擋下的命令；失敗時顯示原因，管理員有「放行」與「重新驗收」。

**命令權限與禁止清單**（重要）：
- 驗收者要啟動伺服器、跑測試，所以**開放 Bash**，另加一份**禁止清單**。你的團隊都是信任的人，這份清單防的是 Claude 誤操作，不是惡意。
- 預設禁止：`sudo`／`su`、`git push`、`npm publish`／`cargo publish`、`gh pr create`／`gh release create`、`docker push`、`ssh`／`scp`／`sftp`、`curl … | sh`／`wget … | bash`、`rm -rf` 指到根目錄／家目錄／上層、`launchctl`／`systemctl`／`defaults write`、`npm install -g`／`pip install --user`。
- 管理員可以維護：畫布頂列「🚫」開啟；**全域預設**（整台 server 一份，可新增、停用、刪除、還原預設）與**這個專案**（只能新增專屬規則，或把全域的某一條在這個專案放行）。改了下一次驗收立即生效，每次異動寫稽核紀錄。
- 規則必須是單一的 `Bash(命令:*)`，不能含逗號或換行。誤擋時報告會列出被擋下的命令，管理員據此放寬。
- **這不是沙箱**：只擋最明顯的誤操作。Claude 若把命令寫進腳本再執行（`bash x.sh`），字串比對攔不住；Bash 理論上仍能碰到工作資料夾以外的檔案。需要真正隔離時要另外用容器或獨立使用者（目前不支援）。

**成本與限制**：
- 用瀏覽器操作很花 token（每一步都回傳頁面結構），中型網頁專案驗一輪估計一兩百萬 token；建議在驗收者的角色設定選較便宜的模型。
- 驗收者不能問人（`claude -p` 非互動）：需要人確認的條件一律標成「需人工確認」並寫原因。原生桌面 App 的畫面也一樣只做建置與啟動。
- Windows 與 Linux 的 Chrome 偵測與埠清理沿用桌面版的寫法，**沒有在這兩個平台實測**。

## Server 技能（自訂技能）

共用看板專案開了「由 server 執行」後，Agent 也能用技能。第一階段支援**自訂技能**（資料庫、系統連線技能見後面的限制）。設計見 `docs/superpowers/specs/2026-11-26-server-skills-custom-design.md`。

**怎麼用**
1. 管理員在共用專案的畫布，把左側「技能」區的「自訂技能」拖到某位 Agent 上，開啟「server 技能庫」對話框。
2. 按「從本機資料夾加入…」選含 `SKILL.md` 的資料夾（技能本身、含多個技能的資料夾、或程式庫根目錄都行）。AITerm 把資料夾上傳到 server 的技能庫（整台 server 共用，不分專案）；同名技能會取代內容，已裝它的 Agent 不受影響。
3. 勾選這位 Agent 要用的技能（最多 12 個）按「儲存」。從下一張卡開始生效。
4. 成員看得到技能庫與每位 Agent 的技能，不能上傳、刪除或設定。

**派工時發生什麼**
- 工程師、審查者、協調者（拆規格）與驗收者的卡都會附技能：提示詞多一段清單（名稱、一句用途、`SKILL.md` 路徑），Agent 需要時才讀全文，跟桌面版一樣。
- 給 Agent 的是**一次性的副本**（`<資料夾>/run/<卡片 id>/skills/`，卡結束就刪），不是技能庫本身，所以工程師就算改了技能檔也影響不到共用的技能。
- 技能庫裡讀不到某個技能時，卡照常處理，提示詞會註明那個技能讀不到。

**上傳規則**（server 端檢查）：根目錄要有 `SKILL.md`；最多 300 個檔、總計 10 MB、單檔 2 MB；路徑不能有 `..`、絕對路徑、反斜線；`.git`、`node_modules`、`.DS_Store`、符號連結會被略過。技能的名稱與說明讀 `SKILL.md` 開頭的 `name`、`description`。

**限制**
- 工程師與審查者在 server 上沒有 Bash（只有專案的驗證指令），所以技能裡的**腳本不能執行**，技能以「指引文字」為主；驗收者有 Bash，可以。
- 不記錄「讀了哪個技能」（桌面版靠 PostToolUse hook，`claude -p` 沒有），畫布上技能不會因為被讀取而亮起來。

## Server 資料庫技能

共用專案的 Agent 在 server 上也能查資料庫。設計見 `docs/superpowers/specs/2026-11-26-server-skills-db-design.md`。

**怎麼用**
1. 管理員在共用專案的畫布，把「資料庫」技能拖到 Agent 上。對話框會列出 server 上已分享的連線。
2. 管理員按「分享本機連線到 server」，把這台電腦上設定好的連線送上去：帳號密碼由 server 加密保存（跟 AI 金鑰同一套），不會交給 Agent，成員也看不到主機與帳號。分享後可以按「測試」讓 server 實際連一次。
3. 勾選這位 Agent 能用的連線、選權限：**唯讀**、**寫入每次核准**、**可寫入直接放行**。下一張卡開始生效。
4. 支援 **PostgreSQL、MySQL、SQLite**。SQL Server、ODBC、DB2 只能在桌面版用，分享時會明講不支援。SQLite 的檔案路徑是 **server 主機上的路徑**。

**權限（在 server 這邊擋，不靠提示詞）**
- 唯讀查詢（SELECT／WITH／SHOW／DESCRIBE／EXPLAIN，一次一句）一律可以。寧可錯擋：被擋下的只會讓 Agent 收到訊息改寫。
- 唯讀模式擋掉所有寫入；直接放行模式照跑；**每次核准**模式下，寫入會**停住**，頂列出現「🗄 N 筆待核准」，管理員按進去看完整 SQL 後核准或拒絕。核准才執行；拒絕的話 Agent 收到拒絕訊息。成員看得到內容但不能決定。
- 等待核准時 Agent 的工具呼叫是掛著的（最久 24 小時）。卡片被中斷或 server 重啟，等核准的寫入一律當作拒絕，不會在沒人看的時候被執行。
- 所有查詢（含被擋下的）都記錄在 server，每次分享、刪除連線與核准／拒絕都寫稽核紀錄。

**運作方式**：server 內建一個只聽 127.0.0.1 的工具端點，每張卡一個隨機 token，卡結束就作廢。Claude Code 透過它使用 `db_list_connections`、`db_list_schemas`、`db_list_tables`、`db_table_schema`、`db_query`。單句查詢最久 60 秒，結果最多 100 KB。

**限制與提醒**
- 連線的密碼集中在 server：server 主機的安全性更重要（不要放不信任的人可以登入的主機）。
- 資料庫帳號請用最小權限：唯讀模式擋的是 SQL 字串比對，最保險的仍是讓資料庫帳號本身沒有寫入權限。
- 查詢記錄裡有完整 SQL（可能含資料內容），只有登入的成員看得到。

## Server 系統連線技能（網頁系統）

共用專案的 Agent 在 server 上也能操作外部**網頁系統**。設計見 `docs/superpowers/specs/2026-11-27-server-skills-systems-design.md`。

**怎麼用**
1. **在你的電腦登入系統**：照桌面版原本的方式，在「系統連線」分頁設定好系統並在它的瀏覽器視窗登入。
2. **分享到 server**（管理員）：在共用專案畫布，把「系統連線」技能拖到 Agent 上 →「分享本機系統到 server」（只分享設定：名稱、起始網址、白名單網域、權限模式）→ 保持那個瀏覽器視窗開著，按「分享 session」。
3. 勾選這位 Agent 能用的系統，儲存。下一張卡開始生效。成員看得到有哪些系統與誰分享了 session，不能改。
- **登入帳密不離開你的電腦**，只有登入後的 cookie 會送到 server，而且只送**屬於白名單網域**的；server 用加密金鑰檔保存（跟 AI 金鑰同一套），API 永遠不回傳 cookie。

**權限模式**：只讀、每次核准、直接放行（規則模式與桌面 App 型連線只在桌面版）。沿用桌面版的寫入閘門：瀏覽器網路層攔下所有非 GET 請求；頂層導覽到白名單外、非 GET 送往白名單外一律拒絕。每個 `sys_*` 動作都要帶 `intent`（讀或寫）。
- 「每次核准」的寫入會停住，頂列「🗄 N 筆待核准」（與資料庫寫入同一個清單）顯示意圖、方法與網址，管理員核准或拒絕；等 5 分鐘沒人決定視為拒絕。Agent 宣告為讀卻送出寫入會標警示。
- 每個動作寫審計紀錄（`GET /v1/systems/{id}/audit`，成員可讀）。

**運作方式**：每張卡、每個系統在 server 上開一個**隔離的無頭 Chrome**（全新設定檔，載入分享的 session），經 Playwright MCP 給 Claude；卡結束就關掉並刪除設定檔。server 重啟時等核准的請求一律當作拒絕。需要 server 主機有 **Chrome** 與 **Node.js**。

**即時預覽、接手與遠端登入**
- **即時預覽**：Agent 在跑時，共用專案頂列出現「👁 N 個瀏覽器執行中」，點進去選一個看它的畫面。**成員唯讀觀看，只有管理員能操作**；每個畫面最多 3 位觀看者，沒人看時不串流。
- **接手**：管理員按「接手」後，Agent 的工具呼叫會收到「使用者接手中」並暫停，你直接用滑鼠鍵盤（含貼上、中文輸入法）操作，做完按「交還給 Agent」。接手者斷線或離開會自動交還，Agent 不會永遠卡住。接手期間的請求不經寫入閘門判定，仍寫審計並標示。
- **遠端登入**（取代「session 過期要回自己電腦重新分享」）：系統對話框按「在 server 上登入」，server 開一個全新的隔離瀏覽器，你在畫面上直接登入（驗證碼、二階段驗證都可以），完成後按「完成並保存 session」，server 取出登入後的 cookie（只留白名單網域的）加密保存並關掉瀏覽器。同一個系統同時只能有一個遠端登入，閒置 15 分鐘自動關閉；**登入畫面只給管理員**。
- **注意：遠端登入時你輸入的帳號密碼會經過 server 的記憶體**（TLS 加密傳輸、不保存）。畫面上會明講；不想這樣就用「分享 session」從自己的電腦匯出，兩種做法都保留。
- 稽核：觀看、接手、交還、遠端登入的開始、完成、取消都寫紀錄（誰、哪個系統、何時）。
- 畫面是 JPEG 串流，不是即時影片，有延遲，不適合需要精細操作的頁面；檔案選擇器、下載、瀏覽器原生彈窗不一定能操作；輸入法組字過程不會同步，只送最後的結果。

**限制與提醒**
- **session 會過期**。偵測到頁面變成登入頁時，Agent 會停下來回報「需要重新分享 session」，不會自己填帳密；你重新登入並再按「分享 session」後重試即可。登入有效期很短（例如 30 分鐘閒置登出）、綁定裝置或 IP、同帳號只允許單處登入的系統不適合。
- cookie 在 server 上等同登入憑證，server 被入侵時可被冒用（不需要密碼、繞過二階段驗證）。請把 server 放在可信任的主機，並讓系統帳號只有必要的權限。
- 用 GET 改資料的舊系統，閘門的網路層攔不到（只剩 `intent` 宣告與審計）；用 POST 做純查詢的系統在「每次核准」會很吵。
- 配方與定時執行只在桌面版。
- Windows、Linux 的 server 沒有實測。

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
- 看板與 Agent 協作可以在 server 上共用與執行（見「共用 Agent 協作與 server 執行」）；編制切換等尚未支援（驗收者見「Server 驗收者」）。
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
