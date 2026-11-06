# 問題收集接入工作看板 — 設計規格

**日期：** 2026-11-06
**狀態：** 已實作（分支 `feat/intake-to-board`，未合併／未 push／未打 tag）；Rust 全套與前端測試通過，尚未用真實 Claude Code 在看板跑過一輪
**前置：** `2026-11-05-intake-queue-design.md`、`2026-11-06-intake-per-project-design.md`（Intake 的 `project_id` 就是看板專案 id）
**範圍：** 只做「Intake → 看板卡 → 完成後回信」。接 Agent 協作桌布是下一步，不在本期。

## 1. 已拍板的決定

| # | 決定 |
|---|---|
| D1 | 只有 AI 初篩為「要改程式」的問題建看板卡；「提問」仍走終端機分頁，行為不變 |
| D2 | 卡片所在欄依派送模式：自動（`queued` 的問題）→ 看板「排隊」欄；手動（`pending` 的問題）→「規劃」欄 |
| D3 | 工作目錄取專案的 `repo_dir`（`project_settings`，先看桌布 Agent 的目錄，再看記住的程式庫）。**專案沒有 `repo_dir` 時不建卡，問題照舊走終端機分頁**（不猜目錄、不報錯） |
| D4 | 完成偵測用**輪詢**：Intake 的 `tick` 每次掃描查「在看板上」的項目對應的卡片；看板不改 |
| D5 | 回信內容 = 卡片 `ai_summary`；沒有就取卡片 `session_path` 對話記錄的最後一段 Agent 回答；再沒有就用 `error_message`／結果文字。附「成功／失敗／已取消」結果。一律經 `filter::redact` |
| D6 | 卡片失敗或取消 → 回信只存草稿，不自動寄出（同現有「驗證沒過不自動寄」） |
| D7 | 看板卡自己有快照與審查，**不再跑** Intake 的 `verify_command` |
| D8 | 每專案一個開關 `route_to_board`（預設開）；關掉就完全維持舊行為 |

## 2. 資料

`intake_items` 新增：
- 狀態 `on_board`：已建卡，等看板處理。不佔用任何分頁。
- 欄位 `board_task_id TEXT`：對應卡片 id。

狀態機新增：

```
pending/queued ──分類為 code_change 且有 repo_dir──> on_board ──卡片 done──> draft（或 done，自動寄出）
                                                        └─卡片消失（刪除／封存）──> pending（標記錯誤）
```

`reject`／`delete` 對 `on_board` 的項目允許（刪除項目不刪卡片；卡片留在看板由使用者處理）。

## 3. 流程（都在 `tick` 裡，`tick_lock` 序列化）

1. **路由階段（在分頁領取之前）：** 對每個 `route_to_board` 開啟、有 `repo_dir` 的專案，取其 `pending`／`queued` 且尚未分類（`kind IS NULL`）的項目：
   - 初篩分類（沿用 `classify`，逾時 30 秒、失敗＝`code_change`），存 `kind`。
   - `question` → 不動，之後由分頁領取（`prepare` 看到已有 `kind` 就不再分類）。
   - `code_change` → 建卡（`create_task`，標題＝主旨，內文＝問題內容＋來源說明；附件複製進卡片資料夾並 `add_attachment`；`queued` 的項目卡片 `move_task` 到排隊欄）→ 項目轉 `on_board` 並記 `board_task_id`。
   - 專案未開啟（registry 沒有）→ 略過，下次再看。
2. **收尾階段：** 對每個 `on_board` 項目讀對應專案的卡片：
   - 卡片不存在或已封存 → 退回 `pending`，`error`＝「看板卡片已不存在」。
   - 卡片 `done` → 組回信（D5），轉 `draft`，再依內外部設定決定是否自動寄出（D6）。
   - 其他狀態 → 不動。
3. **分頁領取階段：** 照舊，只會領到未走看板的項目。

手動模式的 `pending` 項目被路由後變成 `on_board`，使用者要在看板上自己把規劃欄的卡拖進排隊欄；Intake 的「派送」按鈕不再適用於它。

## 4. 邊界

- 建卡失敗（例如卡片寫入出錯）：分類結果已存，項目留在原狀態，之後由終端機分頁領取，不會每 3 秒重問 AI。
- 同一項目不重複建卡：`board_task_id` 與狀態轉換在同一個條件式更新裡認領（`status IN (pending, queued) AND board_task_id IS NULL`）；建卡失敗則不轉狀態，下次重試。
- 建卡成功但轉狀態失敗 → 刪掉剛建的卡，避免孤兒卡。
- 啟動時 `recover_interrupted` 只處理 `running`；`on_board` 不受影響，重啟後繼續輪詢。
- 卡片完成後尚未寄出的項目已是 `draft`／`done`，不會再被收尾階段處理（只看 `on_board`）。

## 5. UI

問題清單裡 `on_board` 的項目顯示「已送看板」標籤與卡片 id；專案設定加 `route_to_board` 開關；i18n 補 zh-TW／en。

## 6. 測試

- 單元：路由條件（有無 `repo_dir`、分類結果、開關）、建卡內容與欄位、附件複製、卡片被刪退回、收尾回信內容選擇（summary／transcript／error）、失敗卡不自動寄。
- 演練（`rehearsal`）：假 claude 跑看板卡，確認問題完成後產生回信草稿。
