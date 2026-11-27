# Server 系統連線的即時預覽、人工接手與遠端登入（M4）

日期：2026-11-27　接續 `2026-11-27-server-skills-systems-design.md`（M3）。

## 已拍板

- 範圍：**遠端登入＋即時預覽＋執行中接手**三項全做。
- 權限：**成員可以唯讀觀看 Agent 執行中的畫面，只有管理員能操作**（接手、輸入）。遠端登入的畫面含帳密輸入，**只給管理員**。
- 遠端登入時輸入的帳密**會經過 server 的記憶體**（走 TLS、不保存，頁面只拿到瀏覽器本來就會送的表單），畫面上要明講；
  原本「從桌面匯出 session」的做法保留，讓使用者自己選。

## 設計

### 畫面串流與輸入（共用 crate `aiterm-systems::live`）

用 CDP：連上瀏覽器、`Target.attachToTarget`（flatten）取得頁面 session、`Page.startScreencast`（JPEG，約 1280×800、品質 60）收畫面，
每個 `Page.screencastFrame` 都要 `Page.screencastFrameAck` 才會送下一張（天然的流量控制）。分頁變了（新視窗、導覽）自動切到最新的頁面。
輸入用 `Input.dispatchMouseEvent`（移動、按下、放開、滾輪）、`Input.dispatchKeyEvent`、`Input.insertText`。
座標用**正規化（0～1）**，由 server 依最近一張畫面的實際尺寸換算，所以 Agent 執行中（Playwright 管著視窗大小）與遠端登入（我們指定大小）都能用同一套。

### 訊息協定（WebSocket，JSON 文字訊息，走 server 既有的 TLS 與成員驗證）

| 方向 | 訊息 |
|---|---|
| server → client | `{"t":"frame","data":"<base64 jpeg>","w":寬,"h":高}`；`{"t":"state","manual":bool,"url":"…","can_operate":bool,"mode":"agent｜login"}`；`{"t":"closed","why":"…"}` |
| client → server（僅管理員，成員的會被忽略並回錯誤） | `{"t":"mouse","type":"move｜down｜up｜wheel","x":0~1,"y":0~1,"button":"left｜right｜middle","dx":…,"dy":…}`；`{"t":"key","type":"down｜up","key","code","text","vk","mods"}`；`{"t":"text","text":"…"}`；`{"t":"takeover","on":true｜false}`（Agent 執行中）；`{"t":"done"}`（遠端登入，存 session）；`{"t":"back"}` |

### Agent 執行中：預覽與接手

- 每個執行中的（卡片, 系統）可以被觀看：`GET /v1/board/projects/{pid}/sys/live`（列出執行中的：卡片、系統、是否接手中；成員可讀）、
  WebSocket `/v1/board/sys/live/{card}/{system}`。第一個觀看者出現才開始串流，最後一個離開就停（沒人看不耗資源）；每個執行最多 3 位觀看者。
- **接手**＝管理員送 `takeover on`：把既有的 `manual` 旗標設起來——閘門改成直接放行（仍寫審計，標示 `manual`），
  Agent 下一次工具呼叫收到「使用者接手中，請稍候」。**交還**＝`takeover off`，Agent 繼續（它的提示詞要求交還後先重新取畫面快照）。
- 管理員斷線或離開時自動交還，避免 Agent 永遠卡住。

### 遠端登入

- 管理員在系統對話框按「在 server 上登入」：`POST /v1/systems/{id}/login/start` 在 server 開一個**全新的隔離 Chrome**（固定 1280×800，開到起始網址），
  `WS /v1/systems/{id}/login/live` 串流並接受輸入。登入完成按「完成並保存 session」（`done`）：server 用 `Storage.getCookies` 取出、
  篩選白名單網域、加密保存（沿用 M3 的 `store_session`），關掉瀏覽器與暫存。`POST …/login/cancel` 放棄。
- 同一個系統同時只能有一個遠端登入；閒置 15 分鐘自動關閉。**只有管理員能開、能看**。
- 稽核：開始、完成、取消、接手、交還都寫 `audit`（誰、哪個系統、何時）。

### 桌面版（AITerm）

- Rust：`live_open`（連上 WebSocket，用既有的憑證指紋釘選）、`live_send`、`live_close`，畫面用事件推給前端。
- 前端 `LiveViewDialog`：顯示串流畫面；滑鼠、滾輪、鍵盤（含中文輸入法的組字結果走 `text`）轉成上面的訊息；管理員才有輸入與「接手／交還」、「完成並保存 session」按鈕；
  成員看到「唯讀」標示。
- 入口：系統對話框的「在 server 上登入」；執行中的 Agent 節點上的「👁 查看」（只在有瀏覽器在跑時出現）。
- 對話框在遠端登入時明講：**「你輸入的帳號密碼會經過 server 的記憶體（加密傳輸、不保存）。不想這樣就用『分享 session』從自己的電腦匯出。」**

### 已知限制

- 畫面是 JPEG 串流，不是即時影片；有延遲（區網通常 100～300ms），不適合需要精細操作的頁面。
- 檔案選擇器、列印、下載、瀏覽器原生彈窗不一定能操作。
- 剪貼簿不通：貼上密碼要靠 `text` 逐字輸入。
- 輸入法的組字過程不會同步，只送最後的結果文字。

### 測試與驗證

- 純邏輯：座標換算、鍵盤事件對應、協定編解碼、權限檢查、閒置逾時。
- 真的 Chrome（測試站）：串流收得到畫面；管理員的點擊真的送出表單（測試站收到 POST）；成員的輸入被拒；接手後 Agent 呼叫回「接手中」、交還後恢復；
  遠端登入：點測試站的登入按鈕 → 完成 → server 保存了 cookie 且之後 Agent 帶著它開頁面；閒置／取消會收乾淨（沒有殘留 Chrome）。
- 真實視窗：用你的 AITerm 實際看畫面、點擊、輸入、接手與登入（需要你在場）。
