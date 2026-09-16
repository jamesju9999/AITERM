# 互動提示自動展開即時窗格 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 偵測到遠端／本機程式送出 Kitty keyboard protocol 的 push（`ESC[>Ps u`）代表它要逐鍵收原始按鍵時，即時窗格自動撐高並隱藏命令列輸入框，讓按鍵直接原始送出；收到 pop（`ESC[<u`）代表結束時自動收回原本高度。

**Architecture:** 在共用的 `useTerminalBlocks` hook 裡用 xterm.js 既有的 `term.parser.registerCsiHandler`（跟現有 OSC 133 處理同一個模式）掛兩個 CSI handler，用 depth 計數器維護一個新的 `isRawKeyboardModeActive` 布林值並回傳；`TerminalView.tsx`／`RemoteTerminalView/index.tsx` 各自比照現有 `isAlternateBuffer` 的用法，把這個新旗標 OR 進「要不要隱藏輸入框、要撐多高」的判斷式。

**Tech Stack:** React 19 + TypeScript, xterm.js 5.5.0（`@xterm/xterm`），Vitest + React Testing Library + jsdom。

---

## 背景文件

設計文件：`docs/superpowers/specs/2026-09-16-interactive-prompt-live-expand-design.md`——實作前先讀過，裡面有完整的實測證據（`claude` CLI 送出的原始位元組）跟取捨理由，這份計畫不重複貼那些理由，只給精確的程式碼改動。

## File Structure

- Modify: `src/hooks/useTerminalBlocks.ts` — 新增偵測邏輯與 `isRawKeyboardModeActive` 回傳值
- Test: `src/hooks/useTerminalBlocks.test.ts` — 新增偵測邏輯的單元測試
- Modify: `src/components/TerminalView.tsx` — 消費新旗標，展開/收回即時窗格
- Test: `src/components/TerminalView.remoteLiveHeight.test.tsx` — 新增本機分頁的展開行為測試（檔名雖然叫 remoteLiveHeight，但既有測試就是混合本機/遠端情境，沿用同一個檔案，不新建檔案分裂測試上下文）
- Modify: `src/components/RemoteTerminalView/index.tsx` — 消費新旗標，展開/收回即時窗格
- Test: `src/components/RemoteTerminalView/index.test.tsx` — 新增遠端分頁的展開行為測試

---

### Task 1: `useTerminalBlocks` 偵測 Kitty keyboard protocol push/pop

**Files:**
- Modify: `src/hooks/useTerminalBlocks.ts:139` (state 宣告), `:218-223` (`clearAllBlocks`), `:239-290` (`finalizeBlock`), `:346-486` (CSI/OSC effect), `:550-560` (回傳物件), `:20-36` (interface)
- Test: `src/hooks/useTerminalBlocks.test.ts`

- [ ] **Step 1: 寫會失敗的測試**

在 `src/hooks/useTerminalBlocks.test.ts` 檔案最後（`describe("useTerminalBlocks", ...)` 區塊結尾的 `}` 之前）加入：

```ts
  describe("isRawKeyboardModeActive（Kitty keyboard protocol push/pop）", () => {
    it("收到 CSI > u（push）後變 true", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));
      expect(result.current.isRawKeyboardModeActive).toBe(false);

      await act(async () => {
        await writeToTerm(term, "\x1b[>5u");
      });

      expect(result.current.isRawKeyboardModeActive).toBe(true);
    });

    it("push 後收到 CSI < u（pop）變回 false", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      await act(async () => {
        await writeToTerm(term, "\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      await act(async () => {
        await writeToTerm(term, "\x1b[<u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(false);
    });

    it("連續兩次 push 只 pop 一次，仍然是 true（深度計數器，不是布林開關）", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      await act(async () => {
        await writeToTerm(term, "\x1b[>5u\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      await act(async () => {
        await writeToTerm(term, "\x1b[<u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      await act(async () => {
        await writeToTerm(term, "\x1b[<u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(false);
    });

    it("同一次 write 裡先 pop 再 push（Ink 類程式每次重繪的實際模式），最終狀態仍是 true", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      await act(async () => {
        await writeToTerm(term, "\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      await act(async () => {
        await writeToTerm(term, "\x1b[<u\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);
    });

    it("clearAllBlocks() 會把卡在 push 狀態的旗標強制歸零", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      await act(async () => {
        await writeToTerm(term, "\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      act(() => {
        result.current.clearAllBlocks();
      });
      expect(result.current.isRawKeyboardModeActive).toBe(false);
    });

    it("finalizeBlock 強制結案時（例如卡住偵測介入）會把卡在 push 狀態的旗標強制歸零", async () => {
      const { result } = renderHook(() => useTerminalBlocks("session-1", term));

      act(() => {
        result.current.submitCommand("claude");
      });
      await act(async () => {
        await writeToTerm(term, "\x1b[>5u");
      });
      expect(result.current.isRawKeyboardModeActive).toBe(true);

      const blockId = result.current.blocks[0].id;
      act(() => {
        result.current.finalizeBlock(blockId, -1);
      });
      expect(result.current.isRawKeyboardModeActive).toBe(false);
    });
  });
```

- [ ] **Step 2: 執行測試確認全部失敗**

Run: `npx vitest run src/hooks/useTerminalBlocks.test.ts`
Expected: 新增的 6 個 test 全部 FAIL，錯誤訊息是 `result.current.isRawKeyboardModeActive` 為 `undefined`（因為 hook 還沒回傳這個欄位），不是 TypeError 之類的意外錯誤。

- [ ] **Step 3: 實作最小改動**

`src/hooks/useTerminalBlocks.ts:20-36`，在 interface 裡加一個欄位（緊接在 `isAlternateBuffer: boolean;` 後面）：

```ts
  isAlternateBuffer: boolean;
  /** true 代表遠端目前處於「要逐鍵收原始按鍵」的協定模式（Kitty keyboard
   *  protocol 的 push/pop，`ESC[>Ps u` / `ESC[<u`）——不是所有互動選單都會
   *  切 alternate screen buffer，這個訊號補上那個縫。見設計文件
   *  docs/superpowers/specs/2026-09-16-interactive-prompt-live-expand-design.md。 */
  isRawKeyboardModeActive: boolean;
```

`src/hooks/useTerminalBlocks.ts:139`，在 `isAlternateBuffer` state 旁邊加：

```ts
  const [isAlternateBuffer, setIsAlternateBuffer] = useState(false);
  const [isRawKeyboardModeActive, setIsRawKeyboardModeActive] = useState(false);
  // Kitty keyboard protocol 的 push/pop 是可疊加的堆疊語意，用深度計數器
  // 而不是布林值，理由見上面 interface 欄位的註解連結的設計文件。
  const rawKeyboardDepthRef = useRef(0);
```

`src/hooks/useTerminalBlocks.ts:218-223`，`clearAllBlocks` 加上重置（安全網，理由見設計文件「安全網：強制重置」一節）：

```ts
  const clearAllBlocks = useCallback(() => {
    blocksRef.current = [];
    setBlocks([]);
    outputStartRef.current?.marker.dispose();
    outputStartRef.current = null;
    rawKeyboardDepthRef.current = 0;
    setIsRawKeyboardModeActive(false);
  }, []);
```

`src/hooks/useTerminalBlocks.ts:239-244`，`finalizeBlock` 開頭（guard 判斷之後，因為只有真的要結案某個 running 區塊時才需要重置）加上同一個安全網：

```ts
  const finalizeBlock = useCallback(
    (blockId: string, exitCode: number, opts?: { clearOnParsed?: boolean }) => {
      const prev = blocksRef.current;
      const target = prev.find((b) => b.id === blockId);
      if (!target || target.status !== "running") return;

      // 安全網：isAlternateBuffer 是每次讀 xterm 當下的 buffer type 算出來的，
      // 天生自我校正；這裡的深度計數器是我們自己維護的狀態，如果指令被強制
      // 結案（斷線、卡住偵測介入）導致最後一個 pop 沒送到，深度會卡在 >0、
      // 即時窗格永遠展開收不回去。任何一個 running 區塊要結案，都代表這個
      // 區塊不再需要原始鍵盤模式了，在這裡強制歸零。
      rawKeyboardDepthRef.current = 0;
      setIsRawKeyboardModeActive(false);

      const endTime = Date.now();
```

`src/hooks/useTerminalBlocks.ts:346-363`，在既有的 `useEffect` 裡、`disposeOsc` 註冊之前加兩個 CSI handler（放在 `onBufferChange()` 呼叫之後、`const disposeOsc = ...` 之前）：

```ts
    onBufferChange();

    // Kitty keyboard protocol push/pop——見 interface 欄位註解連結的設計
    // 文件。prefix 用 `>`/`<`（合法範圍是 xterm.js IFunctionIdentifier 文件
    // 裡的 \x3c-\x3f），final 都是 `u`。跟下面的 OSC 133 handler 用同一個
    // xterm.js parser hook 機制，不用手動掃描原始字串——xterm 自己處理好
    // 一個逃逸序列被拆成兩次 term.write() 呼叫的邊界情況。
    const disposeRawKbPush = term.parser.registerCsiHandler({ prefix: ">", final: "u" }, () => {
      rawKeyboardDepthRef.current += 1;
      setIsRawKeyboardModeActive(true);
      return true;
    });
    const disposeRawKbPop = term.parser.registerCsiHandler({ prefix: "<", final: "u" }, () => {
      rawKeyboardDepthRef.current = Math.max(0, rawKeyboardDepthRef.current - 1);
      setIsRawKeyboardModeActive(rawKeyboardDepthRef.current > 0);
      return true;
    });

    const disposeOsc = term.parser.registerOscHandler(133, (data) => {
```

`src/hooks/useTerminalBlocks.ts:477-480`，cleanup 加上新 handler 的 dispose：

```ts
    return () => {
      disposeBuffer.dispose();
      disposeOsc.dispose();
      disposeRawKbPush.dispose();
      disposeRawKbPop.dispose();
    };
```

`src/hooks/useTerminalBlocks.ts:550-560`，回傳物件加上新欄位：

```ts
  return {
    blocks,
    submitCommand,
    beginTrackedBlock,
    appendOutput,
    setBlockGitInfo,
    isAlternateBuffer,
    isRawKeyboardModeActive,
    termInstance: term,
    finalizeBlock,
    clearAllBlocks,
  };
```

- [ ] **Step 4: 執行測試確認全部通過**

Run: `npx vitest run src/hooks/useTerminalBlocks.test.ts`
Expected: 全部 PASS（新增的 6 個以及原本所有既有測試都要綠燈，不能只看新增的）。

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useTerminalBlocks.ts src/hooks/useTerminalBlocks.test.ts
git commit -m "feat: detect Kitty keyboard protocol push/pop for raw-input prompts"
```

---

### Task 2: `TerminalView.tsx` 消費 `isRawKeyboardModeActive`

**Files:**
- Modify: `src/components/TerminalView.tsx:148-153`（常數）、`:380`（destructure）、`:639-655`（liveRows effect）、`:2099,2133,2152,2161,2183,2190`（JSX 條件）
- Test: `src/components/TerminalView.remoteLiveHeight.test.tsx`

- [ ] **Step 1: 寫會失敗的測試**

在 `src/components/TerminalView.remoteLiveHeight.test.tsx` 裡，找到 mock `useTerminalBlocks` 的地方（回傳固定物件那段），先把它改成可以依測試案例覆寫的版本——把檔案開頭的：

```ts
const useTerminalBlocksCalls: unknown[][] = [];
vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: (...args: unknown[]) => {
    useTerminalBlocksCalls.push(args);
    return {
      blocks: [],
      isAlternateBuffer: false,
      submitCommand: vi.fn(),
      beginTrackedBlock: vi.fn(),
      appendOutput: vi.fn(),
      setBlockGitInfo: vi.fn(),
      finalizeBlock: vi.fn(),
      termInstance: null,
    };
  },
}));
```

改成：

```ts
const useTerminalBlocksCalls: unknown[][] = [];
let mockIsRawKeyboardModeActive = false;
vi.mock("../hooks/useTerminalBlocks", () => ({
  useTerminalBlocks: (...args: unknown[]) => {
    useTerminalBlocksCalls.push(args);
    return {
      blocks: [],
      isAlternateBuffer: false,
      isRawKeyboardModeActive: mockIsRawKeyboardModeActive,
      submitCommand: vi.fn(),
      beginTrackedBlock: vi.fn(),
      appendOutput: vi.fn(),
      setBlockGitInfo: vi.fn(),
      finalizeBlock: vi.fn(),
      termInstance: null,
    };
  },
}));
```

`beforeEach` 那段（`useTerminalBlocksCalls.length = 0;`）旁邊加上重置：

```ts
beforeEach(() => {
  useTerminalBlocksCalls.length = 0;
  mockIsRawKeyboardModeActive = false;
});
```

然後在檔案最後（既有 `describe` 區塊結尾的 `}` 之前）加入新的 `describe`：

```ts
describe("TerminalView 偵測到需要逐鍵原始輸入時展開即時窗格", () => {
  it("isRawKeyboardModeActive 為 true 時隱藏 WarpInput，且不是 alt-screen 那種全螢幕高度", async () => {
    mockIsRawKeyboardModeActive = true;
    const { container } = render(
      <MemoryRouter>
        <LocaleProvider>
          <TerminalView sessionId="s1" tabId="t1" viewTab="terminal" isActive={true} />
        </LocaleProvider>
      </MemoryRouter>,
    );

    await waitFor(() => {
      expect(useTerminalBlocksCalls.length).toBeGreaterThan(0);
    });

    expect(container.querySelector("textarea, input")).toBeFalsy();
  });
});
```

（沿用檔案裡其他測試已經驗證過可以掛載成功的 `TerminalView` props 組合——照抄同檔案裡既有測試呼叫 `render(...)` 那段用的 props，不要自己猜。）

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/components/TerminalView.remoteLiveHeight.test.tsx`
Expected: 新測試 FAIL——因為 `TerminalView.tsx` 目前完全沒有讀取 `isRawKeyboardModeActive`（destructure 都還沒加），WarpInput 仍會照常渲染出來。

- [ ] **Step 3: 實作最小改動**

`src/components/TerminalView.tsx:148-153`，加新常數：

```ts
const MIN_LIVE_ROWS = 3;
const MAX_LIVE_ROWS = 16;
// 介於 MAX_LIVE_ROWS 跟全螢幕高度之間——遠端程式宣告「要逐鍵收原始按鍵」
// （Kitty keyboard protocol push，見 useTerminalBlocks 的
// isRawKeyboardModeActive）但沒有切 alternate screen buffer 時用這個高度。
// 這類提示通常只有十幾行，撐到跟 alt-screen 一樣填滿整個主控端螢幕高度
// 跳動過大，見設計文件 docs/superpowers/specs/2026-09-16-interactive-prompt-live-expand-design.md。
const EXPANDED_LIVE_ROWS = 24;
```

`src/components/TerminalView.tsx:380`，destructure 加欄位：

```ts
  const { blocks, isAlternateBuffer, isRawKeyboardModeActive, submitCommand, beginTrackedBlock, appendOutput, setBlockGitInfo, finalizeBlock } = useTerminalBlocks(
```

`src/components/TerminalView.tsx:639-655`，在既有的「visibleBlockCount 改變就收回 MIN_LIVE_ROWS」effect 後面，新增一個獨立 effect：

```ts
  const [liveRows, setLiveRows] = useState(MIN_LIVE_ROWS);
  useEffect(() => {
    setLiveRows(MIN_LIVE_ROWS);
  }, [visibleBlockCount]);

  // isRawKeyboardModeActive 從 false 變 true：撐到 EXPANDED_LIVE_ROWS，隱藏
  // WarpInput（下面 JSX 會把它跟 isAlternateBuffer OR 在一起判斷）。變回
  // false：收回 MIN_LIVE_ROWS——如果指令其實還在跑且持續有輸出，下面既有
  // 的「running 中的區塊有新輸出就撐到 MAX_LIVE_ROWS」邏輯（onPtyData 內）
  // 會在下一個 chunk 自然把它撐回 MAX_LIVE_ROWS，不需要在這裡特別處理
  // 「使用者回應後指令還沒結束」的情況。isAlternateBuffer 為 true 時
  // liveRows 根本不影響顯示高度（見下面 JSX 的 height 三元判斷式），所以
  // 這裡不需要額外判斷 isAlternateBuffer。
  useEffect(() => {
    setLiveRows(isRawKeyboardModeActive ? EXPANDED_LIVE_ROWS : MIN_LIVE_ROWS);
  }, [isRawKeyboardModeActive]);
```

`src/components/TerminalView.tsx` 的 JSX 部分，把下面幾處的 `isAlternateBuffer` 改成 `(isAlternateBuffer || isRawKeyboardModeActive)`：

第 2099 行附近（卡片列表）：
```tsx
        {!(isAlternateBuffer || isRawKeyboardModeActive) && (
```

第 2133 行附近（即時窗格高度）：
```tsx
            height: isAlternateBuffer ? "calc(100% - 12px)" : `${liveHeightPx}px`,
```
這行**不用改**——`isRawKeyboardModeActive` 走的是 `EXPANDED_LIVE_ROWS` 撐大 `liveHeightPx`，不是 alt-screen 那個 `calc(100% - 12px)` 分支，維持原樣。

第 2152 行附近（overflow）：**不用改**，同上，`isRawKeyboardModeActive` 這個情境仍然要維持 `overflow: clip`（設計文件「展開行為」一節：不新增捲動容器）。

第 2161 行附近（host 高度）：**不用改**，同理，維持 220px 固定高，靠 `liveHeightPx` 撐大外層框。

第 2183 行附近（agentPhase 顯示）：
```tsx
        {!(isAlternateBuffer || isRawKeyboardModeActive) && agentPhase && (
```

第 2190 行附近（WarpInput）：
```tsx
        {!(isAlternateBuffer || isRawKeyboardModeActive) && (
```

- [ ] **Step 4: 執行測試確認通過**

Run: `npx vitest run src/components/TerminalView.remoteLiveHeight.test.tsx`
Expected: 全部 PASS，包含新增與既有測試。

Run: `npx tsc -b`
Expected: 沒有型別錯誤（新增的 destructure 欄位、effect 都要通過型別檢查）。

- [ ] **Step 5: Commit**

```bash
git add src/components/TerminalView.tsx src/components/TerminalView.remoteLiveHeight.test.tsx
git commit -m "feat: expand local terminal live pane on raw-keyboard-mode prompts"
```

---

### Task 3: `RemoteTerminalView/index.tsx` 消費 `isRawKeyboardModeActive`

**Files:**
- Modify: `src/components/RemoteTerminalView/index.tsx:32-33`（常數）、`:129`（destructure）、`:251-254`（liveRows effect）、`:686,727,736,760`（JSX 條件）
- Test: `src/components/RemoteTerminalView/index.test.tsx`

- [ ] **Step 1: 寫會失敗的測試**

先讀 `src/components/RemoteTerminalView/index.test.tsx` 裡既有的 `useTerminalBlocks` mock 寫法（跟 Task 2 一樣，找回傳固定物件的地方），比照 Task 2 Step 1 的做法：加一個可覆寫的 `mockIsRawKeyboardModeActive`，回傳物件裡加上 `isRawKeyboardModeActive: mockIsRawKeyboardModeActive`，`beforeEach` 重置成 `false`。

然後加入新測試：

```ts
describe("RemoteTerminalView 偵測到需要逐鍵原始輸入時展開即時窗格", () => {
  it("isRawKeyboardModeActive 為 true 時隱藏 WarpInput 跟卡片列表", async () => {
    mockIsRawKeyboardModeActive = true;
    const { container } = render(/* 照抄同檔案既有測試的 render(...) 呼叫與 props */);

    await waitFor(() => {
      expect(container.querySelector(".aiterm-remote-terminal__live-frame")).toBeTruthy();
    });

    expect(container.querySelector(".aiterm-remote-terminal__blocks")).toBeFalsy();
  });
});
```

（`render(...)` 呼叫的確切 props/context wrapper 照抄檔案裡其他測試已經驗證能掛載成功的那一份，不要自己猜——這個檔案已經有掛載 `RemoteTerminalView` 的先例。）

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx`
Expected: 新測試 FAIL，`.aiterm-remote-terminal__blocks` 仍然被渲染出來（因為 `isRawKeyboardModeActive` 還沒被消費）。

- [ ] **Step 3: 實作最小改動**

`src/components/RemoteTerminalView/index.tsx:32-33`：

```ts
const MIN_LIVE_ROWS = 3;
const MAX_LIVE_ROWS = 16;
// 同 TerminalView.tsx 的 EXPANDED_LIVE_ROWS，理由一樣：見設計文件
// docs/superpowers/specs/2026-09-16-interactive-prompt-live-expand-design.md。
const EXPANDED_LIVE_ROWS = 24;
```

`src/components/RemoteTerminalView/index.tsx:129`：

```ts
  const { blocks, isAlternateBuffer, isRawKeyboardModeActive, submitCommand, appendOutput, clearAllBlocks } = useTerminalBlocks(
```

`src/components/RemoteTerminalView/index.tsx:251-254`，加一個獨立 effect（緊接在既有的 `liveRows`/`visibleBlockCount` effect 後面）：

```ts
  const [liveRows, setLiveRows] = useState(MIN_LIVE_ROWS);
  useEffect(() => {
    setLiveRows(MIN_LIVE_ROWS);
  }, [visibleBlockCount]);

  // 跟 TerminalView.tsx 同一套邏輯與理由：isAlternateBuffer 為 true 時
  // liveRows 不影響顯示高度（見下面 JSX 的 height 三元判斷式用的是
  // altBufferHeightPx），這裡不需要額外判斷 isAlternateBuffer。
  useEffect(() => {
    setLiveRows(isRawKeyboardModeActive ? EXPANDED_LIVE_ROWS : MIN_LIVE_ROWS);
  }, [isRawKeyboardModeActive]);
```

`src/components/RemoteTerminalView/index.tsx` 的 JSX：

第 686 行附近（卡片列表）：
```tsx
        {!(isAlternateBuffer || isRawKeyboardModeActive) && (
```

第 727 行附近（即時窗格高度）：**不用改**，`isRawKeyboardModeActive` 這個情境走 `liveHeightPx` 分支（跟 `EXPANDED_LIVE_ROWS` 搭配），不是 `altBufferHeightPx`，維持原本三元判斷式。

第 736 行附近（overflow）：**不用改**，維持 `clip`。

第 760 行附近（WarpInput）：
```tsx
      {!(isAlternateBuffer || isRawKeyboardModeActive) && (
```

- [ ] **Step 4: 執行測試確認通過**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx`
Expected: 全部 PASS。

Run: `npx tsc -b`
Expected: 無型別錯誤。

- [ ] **Step 5: Commit**

```bash
git add src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx
git commit -m "feat: expand remote terminal live pane on raw-keyboard-mode prompts"
```

---

### Task 4: 真機驗證

**Files:** 無程式改動，僅驗證。

- [ ] **Step 1: 本機分頁驗證**

跑 `npm run tauri:dev`，開一個新的本機終端機分頁，`cd` 到一個從未被 `claude` CLI 信任過的資料夾（例如 `mktemp -d`），執行 `claude`。

預期：信任提示一出現，命令列輸入框跟已完成的卡片列表立刻消失、即時畫面窗格明顯撐高，方向鍵可以在「No, exit」/「Yes, I trust this folder」間移動、Enter 可以確認。確認後（不論選哪個）畫面收回原本的輸入框模式。

- [ ] **Step 2: 遠端終端機驗證**

用兩台機器（或同一台機器上的兩個 AITerm 視窗，一個當主控端一個當觀看端）建立一個遠端終端機連線，在觀看端對還沒被信任過的資料夾執行 `claude`。

預期：跟 Step 1 完全一樣的展開/收回行為出現在觀看端畫面上。

- [ ] **Step 3: 確認既有全套測試沒有回歸**

Run: `npm run test`
Expected: 全部 PASS。

Run: `npx tsc -b`
Expected: 無型別錯誤。

Run: `npm run lint`
Expected: 無新增的 lint 錯誤。

---

## 已知風險（照抄自設計文件，供實作時提醒自己）

- 只認 Kitty push/pop，不認 `modifyOtherKeys`（`ESC[>4;2m`）——之後如果遇到只靠後者宣告原始鍵盤模式的程式，這次的偵測不會觸發，行為退回目前（不生效，不會誤觸更糟的行為）。
- `EXPANDED_LIVE_ROWS = 24` 是拍板值，沒有從實際各類互動提示的行數統計出來。
