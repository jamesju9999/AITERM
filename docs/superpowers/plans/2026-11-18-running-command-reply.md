# 指令執行中的回覆輸入——實作計畫

規格：`docs/superpowers/specs/2026-11-18-running-command-reply-design.md`

## 第一期
1. `WarpInput`：新增 props `onReply`、`onStop`、`onForceEnd`、`runningCommand`；回覆模式下 `commitCommand` 走 `onReply`、不存歷史；提示列＋按鈕＋樣式＋i18n。測試先寫。
2. `TerminalView`：把最後一張 running 卡片接到上述 props（`writePty`、`finalizeBlock(id,-1)`）。
## 第二期
3. `src/lib/inputPrompt.ts`＋測試。
4. `WarpInput`：`replyPrompt`（kind/options）→ 快速按鈕、遮罩。
5. `TerminalBlockCard`：等待輸入徽章。
6. `TerminalView`：計算最後一列並傳入。
## 收尾
7. tsc、vitest、CHANGELOG、commit。
