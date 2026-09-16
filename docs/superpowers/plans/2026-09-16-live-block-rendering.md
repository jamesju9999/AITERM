# 執行中指令改用卡片渲染 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 「正在執行中」的指令輸出改用跟「已完成」卡片同一套 `readRenderedLines` 渲染，持續更新而不是只在結束時解析一次，讓即時內容自然撐開高度；整批移除 `liveRows`/`liveTopRows`/`scrollToLine` 這套固定裁切+對齊機制。

**Architecture:** `useTerminalBlocks.ts` 的 marker 登記機制（`outputStartRef`，本來只有 Windows 用）改成所有平台通用，`appendOutput` 每次呼叫都用 `requestAnimationFrame` 節流呼叫 `readRenderedLines` 更新 running 區塊的 `renderedLines`。`TerminalView.tsx`／`RemoteTerminalView/index.tsx` 的卡片列表過濾條件拿掉 `status !== "running"`，讓 running 中已有 `renderedLines` 的區塊也用 `TerminalBlockCard` 顯示；真正的 xterm DOM 在非 alt-screen 情境下移到畫面外（保留功能，只是不顯示）。

**Tech Stack:** React 19 + TypeScript, xterm.js 5.5.0, Vitest + React Testing Library。

**背景文件：** `docs/superpowers/specs/2026-09-16-live-block-rendering-design.md`。

---

## File Structure

- Modify: `src/hooks/useTerminalBlocks.ts` — marker 通用化、`appendOutput` 持續更新 `renderedLines`、拿掉 `onPromptStart`/`onUntrackedCommandBoundary`
- Test: `src/hooks/useTerminalBlocks.test.ts`
- Modify: `src/components/TerminalBlockCard.tsx` — running 中顯示持續更新的耗時
- Test: `src/components/TerminalBlockCard.test.tsx`
- Modify: `src/components/TerminalView.tsx` — 拿掉 liveRows 整套機制，xterm DOM 移到畫面外
- Modify: `src/components/RemoteTerminalView/index.tsx` — 同上
- Test: 大幅簡化/刪除 `TerminalView.windowsPromptAlign.test.tsx`、`TerminalView.windowsRunningResetOffset.test.tsx`、`TerminalView.remoteLiveHeight.test.tsx`；新增 running-block-in-card-list 的測試
- Test: `RemoteTerminalView/index.test.tsx` 同上

---

### Task 1: `useTerminalBlocks.ts` — marker 通用化 + 持續更新 `renderedLines`

- [ ] Step 1：在 `TerminalBlock` 建立的三個地方（`submitCommand`、`beginTrackedBlock`、OSC 133 C 的復原路徑）都登記 marker，不再判斷 `hostPlatform === "windows"`。
- [ ] Step 2：`appendOutput` 除了累加 `rawOutput`，用 `requestAnimationFrame` 節流呼叫一個新的內部函式 `updateLiveRenderedLines(blockId)`：讀 `outputStartRef.current`，若 `blockId` 相符，用 `readRenderedLines(term.buffer.active, marker.line, cursorAbsRow+1, cols)` 算出 `renderedLines` 寫回該 block。節流用一個 `useRef<number | null>` 存 rAF id，同一輪只排一次。
- [ ] Step 3：`finalizeBlock` 不用再自己呼叫 `readRenderedLines`／`parseAnsiToRenderedLines`——`renderedLines` 已經是最新的（前面持續更新的結果）；只需要把 rAF 排程取消掉（避免區塊結案後還有一次遲到的更新寫入舊資料）。
- [ ] Step 4：拿掉 `onPromptStart` 參數與呼叫端、拿掉 `onUntrackedCommandBoundary` 參數與呼叫端（連同 `promptEndRef`、跟它們相關的 OSC 133 A/B 分支裡回報絕對列的那段——OSC 133 B 分支本身還在，只是不再呼叫 `onPromptStart`）。
- [ ] Step 5：`clearAllBlocks` 也要取消任何排程中的 rAF。
- [ ] 寫/更新 `useTerminalBlocks.test.ts`：
  - running 中餵幾行輸出（不觸發 D），`renderedLines` 已經非 `undefined` 且內容正確。
  - 不傳 `hostPlatform`（或傳 `"other"`）一樣能拿到 running 中的 `renderedLines`（marker 通用化）。
  - 同一個 rAF 週期內多次 `appendOutput`，`readRenderedLines`（可以 spy `ansiBlockParser` 模組，或用行為驗證：多次 append 之後只 flush 一次，用 `vi.useFakeTimers` + `requestAnimationFrame` 的 polyfill/mock 控制時機）只真正執行一次。
  - 拿掉的參數（`onPromptStart`/`onUntrackedCommandBoundary`）相關的既有測試整批刪除。
- [ ] `npx vitest run src/hooks/useTerminalBlocks.test.ts`，全綠後 commit。

### Task 2: `TerminalBlockCard.tsx` — running 中顯示持續更新的耗時

- [ ] Step 1：`duration` 的計算改成：`block.status === "running"` 時用 `Date.now() - block.startTime`，透過 `setInterval`（例如每 200ms）觸發重新渲染；`endTime` 存在時維持現有算法。
- [ ] Step 2：`exitClass`／`isFailed` 不用改（running 中本來就不會顯示失敗標記）。
- [ ] 寫/更新 `TerminalBlockCard.test.tsx`：running 中的卡片頭顯示會隨時間變化的耗時文字，不顯示 `exit` 標記。
- [ ] `npx vitest run src/components/TerminalBlockCard.test.tsx`，全綠後 commit。

### Task 3：`TerminalView.tsx` — 拿掉 liveRows 整套、xterm 移到畫面外

- [ ] Step 1：刪除 `MIN_LIVE_ROWS`/`MAX_LIVE_ROWS`/`EXPANDED_LIVE_ROWS`、`liveRows`/`liveTopRows`/`desiredLiveRowsRef`/`recomputeLiveGeometry`/`requestLiveRows`/`syncLiveTopRef`/`promptAbsRowRef` 相關的 state、effect、callback。
- [ ] Step 2：卡片列表過濾條件從 `blocks.filter((b) => b.status !== "running" && b.renderedLines)` 改成 `blocks.filter((b) => b.renderedLines)`。
- [ ] Step 3：`isRawKeyboardModeActive`／`isAlternateBuffer` 為 true 時隱藏 WarpInput 的邏輯不變；新增「有 running 中區塊、且不是 alt-screen」時也隱藏 WarpInput 卡片列表以外的即時窗格 DOM（因為現在 running 中的內容走卡片列表顯示，不需要再顯示一個空的即時窗格）。
- [ ] Step 4：`.aiterm-live-frame`/`.aiterm-terminal-root` 在**非** alt-screen 情境下改成移到畫面外（`position: absolute; left: -99999px`，保留原本尺寸），alt-screen 情境維持現狀（撐滿可用高度、正常顯示）。
- [ ] Step 5：running 中的卡片（`TerminalBlockCard`）加 `onClick` 呼叫 `termRef.current?.focus()`。
- [ ] 更新/刪除受影響的既有測試（`TerminalView.windowsPromptAlign.test.tsx`、`TerminalView.windowsRunningResetOffset.test.tsx`、`TerminalView.remoteLiveHeight.test.tsx`）：這些測試的既有斷言（`liveRows`/位移/`scrollToLine`）已經不適用，整份檔案視情況刪除或改寫成驗證「running 中的卡片有沒有正確顯示在列表裡」。
- [ ] 新增測試：running 中一個區塊有 `renderedLines` 時，卡片列表裡看得到它；alt-screen 使用中 xterm 的 DOM 仍然可見、撐滿高度。
- [ ] `npx vitest run src/components/TerminalView` + `npx tsc -b`，全綠後 commit。

### Task 4：`RemoteTerminalView/index.tsx` — 同 Task 3

- [ ] 對應套用 Task 3 的每一步。
- [ ] 更新/刪除 `RemoteTerminalView/index.test.tsx` 裡對應的位移/`scrollToLine`/`liveRows` 測試。
- [ ] `npx vitest run src/components/RemoteTerminalView` + `npx tsc -b`，全綠後 commit。

### Task 5：真機驗證

- [ ] `npm run tauri:dev`（乾淨重啟），本機分頁跑一次 `claude` 信任提示：確認畫面正常顯示、自然撐開高度、方向鍵/Enter 可以操作。
- [ ] 遠端終端機同樣測一次。
- [ ] `npm run test` 全綠、`npx tsc -b` 無錯誤。

---

## 已知風險（照抄自設計文件）

- `readRenderedLines` 是線性掃描不是遞增式的，指令輸出量非常大時執行中每輪節流都要重新掃全部內容。
- 拿掉 `onPromptStart`/`onUntrackedCommandBoundary` 是整批移除的既有機制，若之後發現還有其他消費者依賴需要另外處理（目前查過沒有）。
