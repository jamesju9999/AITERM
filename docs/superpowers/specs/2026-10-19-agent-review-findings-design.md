# 審查意見分級與再審查收斂

日期：2026-10-19　分支：feat/agent-canvas
參考：Anthropic〈Solving code review at Anthropic〉（Claude Code 團隊的 code review 做法）

## 問題

Agent 協作的審核者每個模組最多退回 `MAX_FIX_ROUNDS = 2` 輪。現在的作法有三個浪費輪數的地方：

1. **意見不分輕重。** `review.json` 的 `reasons` 是一串字，風格建議和正確性錯誤混在一起交給修正卡，
   工程師會照單全收，每條都改。文章指出 Agent 作者會對每個小意見再提交一輪，雜訊代價比人類作者高。
2. **再審查會挖舊問題。** 再審查的 diff 已經只有修正那段（修正卡的 `start_sha..end_sha`），
   但提示沒告訴審核者「只確認上一輪的意見、別在沒改到的地方找新問題」。文章統計同一 PR 反覆被標的問題
   約 59% 是第一輪就存在的程式碼。
3. **修正只改被點到的那一行。** 同類問題散在別處，下一輪又被抓到。

## 設計

### 1. review.json 新增 `findings`（分級意見）

```json
{
  "findings": [
    {"severity": "important", "location": "src/timer.ts:42", "issue": "暫停後再開始會重設剩餘秒數；同樣的寫法也出現在 resume()"},
    {"severity": "nit", "issue": "變數名稱 t 不清楚"},
    {"severity": "pre_existing", "location": "src/util.ts:8", "issue": "既有的除以零"}
  ],
  "provides": "…"
}
```

- `severity` 只接受 `important`／`nit`／`pre_existing`，其他值當成無效的 review.json（跟 verify.json 對
  `status` 的嚴格度一致）。
- **判定由 findings 推導**：有任何 `important` 就是 fail，否則 pass，不管 `verdict` 寫什麼
  （同 verify.json「有 fail 項就是 fail」）。審核者寫 fail 但只有 nit → pass，這正是要的效果。
- 有 `findings` 時 `verdict` 可省略。沒有 `findings` 的舊格式照舊：`verdict` 必填、`reasons` 原樣保留。
- `findings` 在 `parse_review_json` 裡轉成 `reasons` 字串，依 important → nit → pre_existing 排序，
  前綴標籤：`[必修]`／`[建議]`／`[既有]`，有 location 時接在標籤後面：`[必修] src/timer.ts:42：…`。
  **下游（`set_verdict`、`review_reasons` 欄位、前端顯示、修正卡、退回計數）全部不用改**：它們看到的還是
  `verdict` + 字串陣列。

各分級的定義寫在審查提示裡：
- 程式碼：important = 正確性錯誤或與需求不符；nit = 風格、命名、可讀性；pre_existing = 不是這次變更造成的問題。
- 規格：important = 缺少關鍵內容、前後矛盾、模糊到無法實作；nit = 用詞與排版；pre_existing 同上。
- important 必須寫 location（檔案:行號，規格寫章節），並寫出同一類問題還出現在哪裡。

### 2. 再審查只確認修正

再審查（`previous` 存在）的提示加上規則：
1. 逐條確認上一輪的 `[必修]` 是否解決；沒解決就再列為 important。
2. 新的 important 只能來自這次修正的 diff。修正沒改到的程式碼裡的問題標 `pre_existing`，
   除非它讓模組需求完全無法達成。
3. 上一輪已經提過的 `[建議]`、`[既有]` 不要再提。

### 3. 修正卡要求同類全修

`fix_body` 說明三種標籤：`[必修]` 一定要修、`[建議]` 可自行判斷、`[既有]` 不屬於這次的範圍不用修；
並要求「每條必修意見都把同一類問題在整個模組裡找出來一併修掉，不要只改被點到的那一行」。

驗收退回的修正卡（`verify_fix_body`）不變：那是功能條件失敗，不是程式碼意見。

## 不做

- 第二位審核者複核（多花 token，先看分級後的效果）。
- 專案自訂 `REVIEW.md`、通過輪數統計。
- 資料庫欄位與前端改動（標籤直接是字串的一部分）。

## 測試

- `parse_review_json`：findings 推導判定（有 important → fail；只有 nit/pre_existing → pass，即使寫 fail）、
  排序與標籤格式、location 可省略、未知 severity 報錯、舊格式相容、BOM。
- `review_body`：初審提示含 findings 格式與三種分級；再審查提示含「只確認修正」三條規則；規格版用規格的分級定義。
- `fix_body`：含標籤說明與同類全修。
- `cargo test --workspace --no-fail-fast` 全綠。
