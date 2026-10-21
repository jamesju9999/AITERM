# Agent 協作：需求先行（打需求 → AI 組隊 → 自動派工）— 設計

日期：2026-10-21
狀態：使用者已核准設計（選項 1：需求框先行＋自動組隊），授權寫完 spec 後直接實作到完成。

## 目標

首次使用 Agent 協作時，貼近直接使用 AI Agent 的體驗：使用者不必先認識 Coder／Reviewer／Orchestrator 再拖上桌布，
只要說「你想做什麼」，系統依需求組好團隊並派出第一張卡。

## 範圍與不變的事

- 只出現在**空桌布**。桌布上一有 Agent，需求框就消失，回到現有流程（模組庫、回報問題）。
- 角色池拖拉、「從團隊範本開始」完全保留；三條路互不影響。
- 不動後端：全部用現有 IPC（`createAgent`、`createTask`、`assignAgent`、`setProjectBrief`、`invokeAiComplete`）。

## 入口（前端）

空桌布中央（取代現在的「把工程師從上方拖進來」那一塊）：

- 多行輸入框「你想做什麼？」＋「送出」按鈕。
- 輸入框下：程式庫列。專案已記住程式庫 → 只顯示路徑與檢查結果（沿用 `RepoStatus`，不是 git／沒 commit 可一鍵初始化）；
  沒有 → 「選擇資料夾…」。程式庫檢查不是 `ok` 時送出鈕停用。
- 沒有可用的 AI 供應商（`listProviders` 為空）時：不阻擋，送出時直接用內建「標準」範本，並在說明裡註明。
- 次要連結「或從團隊範本開始」（原按鈕）；角色池一直在上方。

## 送出流程（新檔 `startFromRequirement.ts`，純函式＋一個編排函式）

1. 簡介：`getProjectBrief` 為空（null／空白）→ `setProjectBrief(requirement)`。有內容則不動。
2. 推薦：把「簡介（若有）＋本次需求」組成給 AI 的描述，呼叫 `invokeAiComplete(buildRecommendSystem, buildRecommendMessage)`，
   用 `parseRecommendation` 驗證。沒有供應商、呼叫失敗、解析失敗 → 退回內建 `standard` 範本，說明欄註明「用了預設團隊」。
3. 建團隊：`setProjectRepo`（若使用者剛選的）→ 對 `toCreate(tpl, [], repo, 40, 40)` 的每個結果 `createAgent`。
4. 第一張卡：`createTask`（標題＝需求第一行，內文＝需求全文；`project_dir` 取接手者的）→ `assignAgent` 給「第一站」。
   - 第一站 `pickFirstStation(agents)`：有 `produces_spec` 的 Coder（需求分析師／規格撰寫師）→ 他；否則負載最小的一般 Coder。
   - 為什麼：產出規格者的流程本來就是先寫規格再由協調者拆解；一般 `pickAssignee` 會排除他們，這裡不能沿用。
5. 在桌布上留一則可關閉的說明「AI 為你組了這支團隊：<summary>＋各角色理由」（或預設團隊的註明）。

### 錯誤處理

- 組隊到一半失敗：已建的 Agent 保留，輸入框內容保留並顯示錯誤；再按送出時，因桌布已有 Agent 需求框已消失，
  所以失敗訊息要提示「團隊已建好，請從模組庫新增需求」。（避免重複建團隊。）
- 建卡／派工失敗：團隊保留，需求文字寫進錯誤訊息旁的「複製」不做；只顯示錯誤。
- 送出中輸入框與按鈕鎖定。

## 測試

- `startFromRequirement.test.ts`（純邏輯）：
  - 簡介為空才寫入；有內容不動；
  - 推薦成功用推薦的範本；失敗／無供應商退回 `standard` 並標註；
  - `pickFirstStation`：有產出規格者 → 他；沒有 → 負載最小的一般 Coder；沒有 Coder → null。
- `index.test.tsx`：空桌布顯示需求框、有 Agent 時不顯示；程式庫未就緒時送出停用；送出後依序呼叫 createAgent → createTask → assignAgent。
- i18n：en／zh-TW 皆補，`i18n.test.ts` 通過。
- 實機驗收（使用者）：新專案空桌布打需求 → 團隊出現、第一張卡派出。

## 刻意不做

- 送出前確認團隊（使用者選「直接開跑，事後可調」）。
- 中途由協調者提議加角色（選項 3，之後另案）。
- 之後的需求寫進簡介。
