# 遠端終端機即時窗格高度與外觀對等 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 `RemoteTerminalView`（遠端觀看者連線時看到的分頁）的即時畫面窗格外觀與高度行為，對齊本機 `TerminalView` 的既有機制——拿掉自動縮放字體塞滿視窗的做法，改成固定字體 + 動態高度（閒置縮小、指令執行中撐高），並讓卡片列表跟即時窗格共用同一個捲動容器、新卡片出現時自動捲到底部。

**Architecture:** 移除 `computeFittingFontSize`/`recomputeFontSize` 這整套自動縮放字體機制；比照 `TerminalView.tsx` 的 `MIN_LIVE_ROWS`/`MAX_LIVE_ROWS` 常數與「外層 wrapper 動態縮放高度、內層 xterm host 本身固定內部大小」的 DOM 結構，新增 `liveRows` 狀態；卡片列表與即時窗格合併成單一個 `overflow-y: auto` 的外層捲動容器，新卡片出現時 `scrollTo` 捲到底部；即時窗格外層套用跟本機一樣的邊框/圓角/高度過場動畫樣式。

**Tech Stack:** TypeScript / React 19 / xterm.js（`RemoteTerminalView/index.tsx`）、CSS（`RemoteTerminalView/index.css`）、Vitest。

**參考設計文件：** `docs/superpowers/specs/2026-08-28-remote-terminal-live-height-parity-design.md`

---

### Task 1: 移除自動縮放字體機制

**Files:**
- Modify: `src/components/RemoteTerminalView/index.tsx`
- Modify: `src/components/RemoteTerminalView/index.test.tsx`

- [ ] **Step 1: 移除 `recomputeFontSizeRef`／`sizeRef` 宣告**

找到：

```ts
  // 後續一個任務會賦值成真正的字級重算函式；這裡先放 ref 讓 xterm 建立
  // 那個 effect（先寫）能呼叫到它，即使賦值它的 effect（後寫）還沒跑。
  const recomputeFontSizeRef = useRef<(() => void) | null>(null);

  // 主控端最後一次告知的尺寸——ResizeObserver 的 callback 要用到「當下」
  // 的 cols/rows，但它是掛載時註冊一次的閉包，不會自動看到後續 Granted/
  // Resize 事件更新的值，所以用 ref 橋接（這個 repo 在 Tauri 事件監聽上
  // 踩過同一類坑）。
  const sizeRef = useRef({ cols: 80, rows: 24 });

  // Granted 事件的 hostOs 空字串代表「這是後續 resize 通知，不是初次
  // 授權」——只有非空字串才更新，讓 hostPlatform 維持第一次拿到的值。
  const [hostPlatform, setHostPlatform] = useState<"windows" | "other">("other");
```

改成（只留 `hostPlatform`）：

```ts
  // Granted 事件的 hostOs 空字串代表「這是後續 resize 通知，不是初次
  // 授權」——只有非空字串才更新，讓 hostPlatform 維持第一次拿到的值。
  const [hostPlatform, setHostPlatform] = useState<"windows" | "other">("other");
```

- [ ] **Step 2: 移除 `recomputeFontSize` 函式與它的兩個 effect**

找到：

```ts
  const recomputeFontSize = useCallback(() => {
    const term = termRef.current;
    const host = hostRef.current;
    if (!term || !host) return;
    const { cols, rows } = sizeRef.current;
    const fitted = computeFittingFontSize(term, cols, rows, host.clientWidth, host.clientHeight);
    if (fitted !== null) term.options.fontSize = fitted;
  }, []);

  useEffect(() => {
    recomputeFontSizeRef.current = recomputeFontSize;
  }, [recomputeFontSize]);

  useEffect(() => {
    if (!hostRef.current) return;
    const ro = new ResizeObserver(() => recomputeFontSizeRef.current?.());
    ro.observe(hostRef.current);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
```

改成（整段移除，只留最後那個 `useEffect(() => {` 開頭，那是掛事件監聽的既有 effect，不動它）：

```ts
  useEffect(() => {
```

- [ ] **Step 3: `onShareViewerGranted` 的 handler 裡移除 `sizeRef`/`recomputeFontSizeRef` 呼叫**

找到：

```ts
    track(
      onShareViewerGranted(connId, ({ mode, cols, rows, hostOs }) => {
        // 尺寸由主控端說了算——照它給的建立，不用自己的視窗大小。
        // `mode` 為空字串代表這是後續的 resize 通知，不是初次核准。
        if (mode) setPhase({ kind: "live", mode });
        if (hostOs) setHostPlatform(hostOs === "windows" ? "windows" : "other");
        const term = termRef.current;
        if (term && cols > 0 && rows > 0) {
          term.resize?.(cols, rows);
          sizeRef.current = { cols, rows };
          recomputeFontSizeRef.current?.();
        }
      }),
    );
```

改成：

```ts
    track(
      onShareViewerGranted(connId, ({ mode, cols, rows, hostOs }) => {
        // 尺寸由主控端說了算——照它給的建立，不用自己的視窗大小。
        // `mode` 為空字串代表這是後續的 resize 通知，不是初次核准。
        if (mode) setPhase({ kind: "live", mode });
        if (hostOs) setHostPlatform(hostOs === "windows" ? "windows" : "other");
        const term = termRef.current;
        if (term && cols > 0 && rows > 0) {
          term.resize?.(cols, rows);
        }
      }),
    );
```

- [ ] **Step 4: `onFontChanged` 裡移除 `recomputeFontSizeRef` 呼叫**

找到：

```ts
    const onFontChanged = (e: Event) => {
      const { fontSize, fontFamily } = (e as CustomEvent).detail as { fontSize: number; fontFamily: string };
      term.options.fontSize = fontSize;
      term.options.fontFamily = fontFamily;
      // 字型/字級變了，同樣的 cols×rows 需要的容器空間也變了——Task 4
      // 加的 recomputeFontSize 會在這個函式存在後接手這件事。
      recomputeFontSizeRef.current?.();
    };
```

改成：

```ts
    const onFontChanged = (e: Event) => {
      const { fontSize, fontFamily } = (e as CustomEvent).detail as { fontSize: number; fontFamily: string };
      term.options.fontSize = fontSize;
      term.options.fontFamily = fontFamily;
    };
```

- [ ] **Step 5: 移除 `computeFittingFontSize` 函式定義**

找到檔案最後面（`endReasonText` 函式之後）：

```ts
/**
 * 量測 `term` 目前字級下的字元格 CSS 像素尺寸，線性外推到「剛好能讓
 * `cols`×`rows` 塞進 `containerWidth`×`containerHeight`」的字級。
 *
 * 用線性外推而不是二分搜尋反覆調字級再量測：等寬字型的字元格尺寸本來就
 * 跟字級成正比（CSS `font-size` 的定義就是線性縮放），量一次目前字級的
 * 尺寸就能直接算出答案，不需要迭代。
 *
 * 量測用的 `_core._renderService.dimensions.css.cell` 是 xterm.js 沒有
 * 公開的內部欄位——這個 repo 已經有先例（`TerminalView.tsx` 算
 * `liveHeightPx` 用的是同一條路徑），這裡沿用同樣的 escape hatch。量不到
 * 時（例如字型還沒載入完成）回傳 `null`，呼叫端維持目前字級不做任何事。
 *
 * 回傳值夾在 [8, 32] 之間：8 是「還能看得清楚」的下限（spec 用語：最小
 * 可讀字級），塞不下的話交給 CSS 捲軸，不繼續往下縮；32 純粹是防呆上限，
 * 避免極端情況（例如容器剛好非常大、cols/rows 很小）算出離譜的字級。
 *
 * **已知限制**：這個公式假設字元格尺寸跟字級完全線性縮放。瀏覽器的字型
 * hinting/像素對齊在小字級時可能讓量測值輕微偏離線性，理論上外推結果
 * 可能有 1px 等級的誤差——實務上影響很小（沒有回饋迴圈去修正），先不處理。
 */
function computeFittingFontSize(
  term: Terminal,
  cols: number,
  rows: number,
  containerWidth: number,
  containerHeight: number,
): number | null {
  const dims = (
    term as unknown as {
      _core?: { _renderService?: { dimensions?: { css?: { cell?: { width?: number; height?: number } } } } };
    }
  )._core?._renderService?.dimensions?.css?.cell;
  if (!dims?.width || !dims?.height || cols <= 0 || rows <= 0) return null;

  const currentFontSize = term.options.fontSize ?? 14;
  const maxByWidth = (containerWidth * currentFontSize) / (cols * dims.width);
  const maxByHeight = (containerHeight * currentFontSize) / (rows * dims.height);
  const fitted = Math.floor(Math.min(maxByWidth, maxByHeight));
  return Math.max(8, Math.min(fitted, 32));
}
```

整段刪除（檔案應該在 `endReasonText` 函式結尾的 `}` 之後直接結束）。

- [ ] **Step 6: 修正測試檔裡已經過時的 `FakeResizeObserver` 註解**

找到 `src/components/RemoteTerminalView/index.test.tsx` 裡的：

```ts
// jsdom doesn't implement ResizeObserver; RemoteTerminalView's font-fitting
// effect calls it unconditionally on mount (same pattern as TerminalView's
// tests).
class FakeResizeObserver {
```

改成（這個 shim 保留——不確定有沒有其他依賴仍隱性需要全域 `ResizeObserver` 存在，
拿掉風險大於留著；只更正註解，不再說是這次移除的字級縮放效果在用）：

```ts
// jsdom doesn't implement ResizeObserver — kept as a no-op global shim in
// case something else in the render tree still expects it to exist
// (harmless if nothing does).
class FakeResizeObserver {
```

- [ ] **Step 7: 確認 tsc 沒有因為移除這些程式碼報錯，且既有測試全數通過**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx`
Expected: 全數通過（既有 13 個測試，這個 Task 沒有新增測試——純移除，不影響任何
既有測試斷言的行為）。

**關於 spec 提到的「字體大小應該直接反映 localStorage 設定值」這一點**：這個檔案
的測試把整個 `@xterm/xterm` mock 掉，mock 的 `Terminal` 類別沒有建構子（見
`index.test.tsx` 裡的 `vi.mock("@xterm/xterm", ...)`），`new Terminal({...})` 傳
進去的建構參數（含 `fontSize`）會被靜靜忽略、量不到——現有的測試基礎設施沒有
辦法自動化驗證這件事，寫一個新測試需要先改動這個 mock 讓建構子真的接住參數，
影響到其他 13 個既有測試的 mock 基礎設施，風險大於這個 Task 的範圍。改成用讀
程式碼的方式驗證：確認 Step 1-4 只移除了「覆蓋」`term.options.fontSize` 的邏輯
（`recomputeFontSize`、`onShareViewerGranted`/`onFontChanged` 裡呼叫它的部分），
沒有動到掛載時讀 `localStorage.getItem("aiterm-font-size")` 設進
`initFontSize`、傳進 `new Terminal({ fontSize: initFontSize, ... })` 那幾行—— 
`git diff` 這個檔案，確認這幾行完全沒有出現在變更範圍內。

- [ ] **Step 8: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx
git commit -m "$(cat <<'EOF'
refactor(remote-terminal): 移除自動縮放字體塞滿視窗的機制

跟本機終端機比較後發現：遠端終端機用「量測字元格尺寸、線性外推出能讓
主控端 cols×rows 網格塞進容器的字級」這套機制動態縮放字體，是即時
畫面高度固定、體驗跟本機終端機不一致的根本原因之一。這個 Task 先拿掉
這整套機制（computeFittingFontSize/recomputeFontSize/對應的
ResizeObserver），後續 Task 改用跟本機同一套動態高度機制取代。

字體大小改回單純反映使用者設定（掛載時讀 localStorage、
aiterm:font-changed 事件套用新值），不再被縮放邏輯覆蓋——這部分程式碼
本來就存在，只是先前一直被縮放蓋掉。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: 即時窗格動態高度、卡片與即時窗格合併捲動容器、自動捲到底部、視覺樣式

**Files:**
- Modify: `src/components/RemoteTerminalView/index.tsx`
- Modify: `src/components/RemoteTerminalView/index.css`
- Test: `src/components/RemoteTerminalView/index.test.tsx`

- [ ] **Step 1: 寫失敗的測試**

在 `src/components/RemoteTerminalView/index.test.tsx` 裡，緊接既有的
`"decodes incoming PTY bytes as UTF-8 and feeds them into appendOutput, not
just onto the xterm screen"` 測試之後、`describe("disconnect timing
(StrictMode dev-mode trap)", ...)` 之前，新增：

```tsx
  it("即時窗格在指令執行中撐到最大高度、指令完成變成卡片後收回最小高度", async () => {
    const { container } = render(<RemoteTerminalView tabId="t1" connId="c12" sas="1212" isActive />);
    await waitFor(() => expect(handlers["granted:c12"]).toBeDefined());
    handlers["granted:c12"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);

    const textarea = await screen.findByPlaceholderText(/輸入指令|Type a command/i);
    await waitFor(() => expect(textarea).not.toBeDisabled());
    await userEvent.type(textarea, "echo hi{Enter}");

    // 指令送出後，模擬 shell 真的產生了一批輸出——即時窗格應該撐到最大
    // 高度（測試環境量不到 xterm 真正的字元格尺寸，會落到 14*1.1 的
    // fallback，MAX_LIVE_ROWS=16 對應 Math.round(16*14*1.1) = 246px）。
    await waitFor(() => expect(handlers["data:c12"]).toBeDefined());
    act(() => {
      handlers["data:c12"](btoa("hi\r\n") as never);
    });

    const liveFrame = () => container.querySelector(".aiterm-remote-terminal__live-frame") as HTMLElement;
    await waitFor(() => {
      expect(liveFrame().style.height).toBe("246px");
    });

    // 指令執行完畢、變成卡片——即時窗格應該收回最小高度
    // （MIN_LIVE_ROWS=3 對應 Math.round(3*14*1.1) = 46px）。
    await waitFor(() => expect(capturedOscHandler).toBeTruthy());
    act(() => {
      capturedOscHandler!("D;0");
    });
    await waitFor(() => {
      expect(liveFrame().style.height).toBe("46px");
    });
  });

  it("新卡片出現時自動捲動到最底部", async () => {
    const scrollToSpy = vi.spyOn(HTMLElement.prototype, "scrollTo").mockImplementation(() => {});
    try {
      render(<RemoteTerminalView tabId="t1" connId="c13" sas="1313" isActive />);
      await waitFor(() => expect(handlers["granted:c13"]).toBeDefined());
      handlers["granted:c13"]({ mode: "control", cols: 80, rows: 24, hostOs: "linux" } as never);

      const textarea = await screen.findByPlaceholderText(/輸入指令|Type a command/i);
      await waitFor(() => expect(textarea).not.toBeDisabled());
      await userEvent.type(textarea, "echo hi{Enter}");

      // 只關心指令完成、卡片出現那一刻的呼叫，清掉掛載/送出指令過程中
      // 可能發生的其他呼叫。
      scrollToSpy.mockClear();

      await waitFor(() => expect(capturedOscHandler).toBeTruthy());
      act(() => {
        capturedOscHandler!("D;0");
      });

      await waitFor(() => expect(screen.getByText("echo hi")).toBeInTheDocument());
      expect(scrollToSpy).toHaveBeenCalled();
    } finally {
      scrollToSpy.mockRestore();
    }
  });
```

- [ ] **Step 2: 執行測試，確認失敗**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx -t "即時窗格在指令執行中\|新卡片出現時自動捲動"`
Expected: FAIL——目前完全沒有 `.aiterm-remote-terminal__live-frame` 這個 class、
`liveRows` 這個狀態，也沒有任何 `scrollTo` 呼叫，兩個測試都會找不到元素或斷言
失敗。

- [ ] **Step 3: 新增常數與 `blocksRef` 橋接**

找到 `src/components/RemoteTerminalView/index.tsx` 頂部：

```ts
import "./index.css";

interface Props {
```

改成：

```ts
import "./index.css";

const MIN_LIVE_ROWS = 3;
const MAX_LIVE_ROWS = 16;

interface Props {
```

找到：

```ts
  const { blocks, submitCommand, appendOutput, clearAllBlocks } = useTerminalBlocks(
    connId,
    termState,
    undefined,
    undefined,
    undefined,
    undefined,
    write,
    hostPlatform,
  );
  const clearAllBlocksRef = useRef(clearAllBlocks);
  clearAllBlocksRef.current = clearAllBlocks;
  // 同一顆橋接 ref、同一個理由：`onShareViewerData` 訂閱只依賴 [connId]，
  // 不想為了這個值重新訂閱一次所有 share-viewer://* 事件。
  const appendOutputRef = useRef(appendOutput);
  appendOutputRef.current = appendOutput;

  const [aiUnsupported, setAiUnsupported] = useState(false);
```

改成（新增 `blocksRef`，以及即時窗格高度需要的狀態與計算）：

```ts
  const { blocks, submitCommand, appendOutput, clearAllBlocks } = useTerminalBlocks(
    connId,
    termState,
    undefined,
    undefined,
    undefined,
    undefined,
    write,
    hostPlatform,
  );
  const clearAllBlocksRef = useRef(clearAllBlocks);
  clearAllBlocksRef.current = clearAllBlocks;
  // 同一顆橋接 ref、同一個理由：`onShareViewerData` 訂閱只依賴 [connId]，
  // 不想為了這個值重新訂閱一次所有 share-viewer://* 事件。
  const appendOutputRef = useRef(appendOutput);
  appendOutputRef.current = appendOutput;
  // `onShareViewerData` 的 handler 同樣只依賴 [connId] 註冊一次，裡面要
  // 判斷「現在有沒有指令在跑」需要讀到最新的 `blocks`，不橋接的話會是
  // 掛載當下那份、永遠看不到後續指令建立的新區塊（跟上面兩顆 ref 同一個
  // 理由，TerminalView.tsx 的 blocksRef 也是同樣的橋接）。
  const blocksRef = useRef(blocks);
  useEffect(() => {
    blocksRef.current = blocks;
  }, [blocks]);

  const [aiUnsupported, setAiUnsupported] = useState(false);

  // 卡片列表跟即時窗格共用同一個外層捲動容器，新卡片完成渲染時捲到底部
  // ——跟 TerminalView.tsx 的 blockListRef 同一個手法、同一個理由。
  const scrollAreaRef = useRef<HTMLDivElement | null>(null);
  const visibleBlockCount = blocks.filter((b) => b.status !== "running" && b.renderedLines).length;
  useEffect(() => {
    scrollAreaRef.current?.scrollTo({ top: scrollAreaRef.current.scrollHeight });
  }, [visibleBlockCount]);

  // 即時窗格閒置時縮到 MIN_LIVE_ROWS、有指令在跑時撐到 MAX_LIVE_ROWS——
  // 跟 TerminalView.tsx 完全同一套機制、同一組數值：閒置時只顯示提示
  // 字元不需要佔用大片空間，指令執行中撐開避免輸出被裁掉（滑鼠滾輪跟
  // liveRows 毫無關聯，裁掉了就拿不回來），指令完成變成卡片
  // （visibleBlockCount 改變）後收回最小高度。
  const [liveRows, setLiveRows] = useState(MIN_LIVE_ROWS);
  useEffect(() => {
    setLiveRows(MIN_LIVE_ROWS);
  }, [visibleBlockCount]);

  // xterm.js 沒有公開 API 可以讀字元格高度——這裡讀的是跟 TerminalView.tsx
  // 同一個內部欄位，同一個 escape hatch，這個 repo 已經有先例。
  const cellHeightPx =
    (termState as unknown as { _core?: { _renderService?: { dimensions?: { css?: { cell?: { height?: number } } } } } } | null)
      ?._core?._renderService?.dimensions?.css?.cell?.height || 14 * 1.1;
  const liveHeightPx = Math.round(liveRows * cellHeightPx);
```

- [ ] **Step 4: 修正 `onShareViewerData` 的 handler——`appendOutput` 搬進 `write()` 完成 callback，並加上撐高判斷**

找到：

```ts
    track(
      onShareViewerData(connId, (b64) => {
        const bytes = atob(b64);
        const arr = new Uint8Array(bytes.length);
        for (let i = 0; i < bytes.length; i++) arr[i] = bytes.charCodeAt(i);
        termRef.current?.write(arr);
        // 分段卡片的內容是從這批位元組解析出來的（跟畫面同一份資料）——
        // 不接這行的話，卡片永遠只有指令文字跟耗時，看不到任何輸出內容。
        // 用跟本機分頁一樣的 stream decoder，不要對 `bytes`（atob 的
        // Latin1-per-byte 字串）直接呼叫 appendOutput：那樣多位元組 UTF-8
        // 字元會被拆散成亂碼。
        appendOutputRef.current(decoder.decode(arr, { stream: true }));
      }),
    );
```

改成：

```ts
    track(
      onShareViewerData(connId, (b64) => {
        const bytes = atob(b64);
        const arr = new Uint8Array(bytes.length);
        for (let i = 0; i < bytes.length; i++) arr[i] = bytes.charCodeAt(i);
        // 實機測試抓到的 bug（跟 TerminalView.tsx 那次 appendOutput race
        // fix 是同一個根因）：xterm.js 的 write() 對不是緊跟著「使用者剛
        // 輸入」的資料，一律用 setTimeout 排到下一輪事件迴圈才真正解析，
        // 不是呼叫當下就同步跑完。appendOutput 若在 write() 呼叫之後就
        // 同步執行，一次湧入多個 chunk 時內容會在對應的區塊還沒真正建立
        // 好之前就被略過、永遠救不回來。改成搬進 write() 的完成 callback，
        // 保證同一個 chunk 已經先被處理過。
        termRef.current?.write(arr, () => {
          // 分段卡片的內容是從這批位元組解析出來的（跟畫面同一份資料）
          // ——不接這行的話，卡片永遠只有指令文字跟耗時，看不到任何輸出
          // 內容。用跟本機分頁一樣的 stream decoder，不要對 `bytes`
          // （atob 的 Latin1-per-byte 字串）直接呼叫 appendOutput：那樣
          // 多位元組 UTF-8 字元會被拆散成亂碼。
          appendOutputRef.current(decoder.decode(arr, { stream: true }));
          // 跟 TerminalView.tsx 同一套機制：有一個追蹤中的區塊還在
          // running，代表指令正在執行、正在產生輸出，即時窗格撐到最大
          // 高度。
          const latestBlock = blocksRef.current[blocksRef.current.length - 1];
          if (latestBlock?.status === "running") {
            setLiveRows(MAX_LIVE_ROWS);
          }
        });
      }),
    );
```

- [ ] **Step 5: 調整 JSX，合併卡片列表與即時窗格成單一捲動容器，即時窗格外層動態縮放高度**

找到：

```tsx
      {/* 分段卡片：跟本機分頁同一套過濾條件（只顯示已結束且已完成 ANSI
          解析的），複用 TerminalBlockCard——不傳 onAskAi，Ask AI 按鈕本身
          是 `{isFailed && onAskAi && (...)}` 條件渲染，不傳就不會出現；
          block.gitInfo 永遠是 undefined（這裡從不呼叫 setBlockGitInfo），
          git 徽章同理自然不出現。 */}
      <div className="aiterm-remote-terminal__blocks">
        {blocks
          .filter((b) => b.status !== "running" && b.renderedLines)
          .map((b) => (
            <TerminalBlockCard
              key={b.id}
              block={b}
              onBookmark={(command) => addBookmark(command)}
              onCopy={(command) => navigator.clipboard.writeText(command).catch(console.error)}
            />
          ))}
      </div>

      <div className="aiterm-remote-terminal__screen">
        <div className="aiterm-remote-terminal__scroll" ref={hostRef} />
      </div>
```

改成：

```tsx
      {/* 卡片列表跟即時窗格共用這個外層捲動容器（跟 TerminalView.tsx 的
          blockListRef 同一個結構）——不再是各自獨立、各自有高度上限的
          兩塊，卡片可以無限往下累積，捲動邊界只有這一層。 */}
      <div className="aiterm-remote-terminal__scroll-area" ref={scrollAreaRef}>
        {/* 分段卡片：跟本機分頁同一套過濾條件（只顯示已結束且已完成 ANSI
            解析的），複用 TerminalBlockCard——不傳 onAskAi，Ask AI 按鈕
            本身是 `{isFailed && onAskAi && (...)}` 條件渲染，不傳就不會
            出現；block.gitInfo 永遠是 undefined（這裡從不呼叫
            setBlockGitInfo），git 徽章同理自然不出現。 */}
        <div className="aiterm-remote-terminal__blocks">
          {blocks
            .filter((b) => b.status !== "running" && b.renderedLines)
            .map((b) => (
              <TerminalBlockCard
                key={b.id}
                block={b}
                onBookmark={(command) => addBookmark(command)}
                onCopy={(command) => navigator.clipboard.writeText(command).catch(console.error)}
              />
            ))}
        </div>

        {/* 外層框住並裁切即時畫面；hostRef 本身內部永遠固定高度，
            這樣它（以及 xterm 自己內部的尺寸監聽）永遠不會因為這一層
            高度變化而看到容器尺寸改變——只有這一層的高度會變。跟
            TerminalView.tsx 的 .aiterm-live-frame 完全同一套機制。 */}
        <div
          className="aiterm-remote-terminal__live-frame"
          style={{
            height: `${liveHeightPx}px`,
            width: "calc(100% - 16px)",
            margin: "6px 8px",
            boxSizing: "border-box",
            flexShrink: 0,
            overflow: "clip",
          }}
        >
          <div
            className="aiterm-remote-terminal__scroll"
            ref={hostRef}
            style={{ height: "220px", width: "100%", boxSizing: "border-box" }}
          />
        </div>
      </div>
```

- [ ] **Step 6: 更新 CSS**

找到 `src/components/RemoteTerminalView/index.css` 裡的：

```css
.aiterm-remote-terminal__screen {
  flex: 1;
  min-height: 0;
  overflow: clip;
  display: flex;
}

/* 字級縮到最小（8px）仍塞不下 cols×rows 時，捲軸只開在這一層——外層
   `.aiterm-remote-terminal`/`.aiterm-remote-terminal__screen` 維持
   `overflow: clip`。整層改用 `overflow: auto` 會重現這個 repo 踩過的
   「貼上內容被瀏覽器捲出視野變空白」那個坑（見 .aiterm-live-frame 的
   同類註解）。 */
.aiterm-remote-terminal__scroll {
  flex: 1;
  min-width: 0;
  overflow: auto;
}

.aiterm-remote-terminal__blocks {
  overflow-y: auto;
  flex-shrink: 0;
  max-height: 40%;
}
```

改成：

```css
/* 卡片列表跟即時窗格共用的外層捲動容器——取代原本各自獨立、卡片區塊
   還有 40% 高度上限的兩塊。跟 TerminalView.tsx 的 blockListRef 外層
   容器同一個結構。 */
.aiterm-remote-terminal__scroll-area {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
}

.aiterm-remote-terminal__blocks {
  flex: none;
}

/* 固定字體後，主控端的欄寬換算成實際像素可能比觀看端視窗窄或寬——這是
   觀看端架構上無法避免的限制（PTY 的實際寬度由主控端決定，觀看端只能
   選擇縮小字體去湊，或固定字體、寬度不夠就橫向捲動，這裡選後者）。
   `overflow: auto`（不是 clip）維持原本這行既有的設定不動，這次沒有
   改它。

   **已知、這次沒有解決的風險**：`.aiterm-live-frame`／`.aiterm-terminal-root`
   （本機終端機的即時窗格）刻意改用 `overflow: clip` 而不是 `hidden`／
   `auto`，是因為實測過 `hidden` 仍會被瀏覽器捲動（例如貼上內容時捲去
   顯示 xterm 內部取得焦點的組字輔助元素），導致畫面看起來空白／IME
   組字被捲到別處。`.aiterm-remote-terminal__scroll` 這裡用 `auto`（維持
   既有設定，不是這次新增的），理論上可能遇到同一類問題——但 `clip`
   不建立捲動容器，會讓「固定字體、寬度不夠時橫向捲動看完整內容」這個
   這次明確要達成的目標完全做不到（本機終端機不需要處理這個矛盾，因為
   FitAddon 保證內容永遠剛好貼合視窗寬度，從來不需要水平捲動）。這個
   衝突沒有簡單的兩全其美解法，這次維持現狀（`auto`），如果之後實機
   測試真的踩到 IME／貼上內容被瀏覽器捲動跑掉的問題，需要另外開一輪
   設計討論，不在這次範圍內。 */
.aiterm-remote-terminal__scroll {
  overflow: auto;
}

/* 跟 TerminalView.css 的 .aiterm-live-frame 完全相同：細邊框、圓角、
   高度變化的平滑過場動畫。 */
.aiterm-remote-terminal__live-frame {
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 8px;
  transition: height 0.12s ease-out;
}
```

- [ ] **Step 7: 執行測試，確認新測試通過，且沒有弄壞既有測試**

Run: `npx vitest run src/components/RemoteTerminalView/index.test.tsx`
Expected: 全數通過（既有 13 個 + 這個 Task 新增的 2 個 = 15 個）。

- [ ] **Step 8: 確認 tsc 沒有報錯**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

- [ ] **Step 9: Commit**

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity
git add src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.css src/components/RemoteTerminalView/index.test.tsx
git commit -m "$(cat <<'EOF'
feat(remote-terminal): 即時窗格改成動態高度，比照本機終端機體驗

實機截圖回報：遠端終端機分頁的即時畫面高度固定不變（卡片區塊 40% 上限
之外的空間全部被即時窗格吃光），提示字元下方留了一大片沒用到的空白，
跟本機終端機「閒置縮小、忙碌撐高」的體驗不一致。

比照 TerminalView.tsx 的 MIN_LIVE_ROWS(3)/MAX_LIVE_ROWS(16) 機制：外層
wrapper 動態縮放高度、內層 xterm host 本身維持固定內部大小，指令執行中
撐到最大、完成變成卡片後收回最小。卡片列表跟即時窗格合併成單一個
overflow-y:auto 的外層捲動容器（原本是各自獨立、卡片區塊還有 40% 高度
上限的兩塊），新卡片出現時自動捲到底部（使用者這次額外提出的需求）。
即時窗格外層加上跟本機一樣的邊框/圓角/高度過場動畫。

順便修正 onShareViewerData 裡一個潛在的 race condition：appendOutput
原本在 term.write() 呼叫之後就同步執行，跟今天稍早在 TerminalView.tsx
修的 appendOutput race 是同一個根因（xterm.js 對非使用者輸入觸發的資料
一律非同步排程解析）——搬進 write() 的完成 callback，順便讓新增的
「有指令在跑就撐高」判斷讀到正確的區塊狀態。

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: 完整驗證與重啟 dev build

**Files:** 無新增/修改，純驗證。

- [ ] **Step 1: 前端型別檢查**

Run: `npx tsc -b`
Expected: 無錯誤輸出，結束碼 0。

- [ ] **Step 2: 前端完整測試套件**

Run: `npx vitest run`
Expected: 全數通過，沒有既有測試被改壞。

- [ ] **Step 3: Lint 範圍比對**

Run: `npx eslint src/components/RemoteTerminalView/index.tsx src/components/RemoteTerminalView/index.test.tsx`
Expected: 沒有新增的 lint 錯誤（若有既有、跟這次改動無關的錯誤，比對是否在
改動前就已存在，方法同這個分支先前幾次驗證：另開一個 disposable 的
`git worktree add --detach` 比對改動前的版本，不要對這個作用中的 worktree
執行 `git checkout <舊commit> -- .`）。

- [ ] **Step 4: 重新啟動 dev build，準備讓使用者做真機測試**

```bash
ps aux | grep -i "tauri dev\|target/debug/app\|node.*vite" | grep -v grep
```

把列出的舊 process（`npm exec tauri dev`、`node .../tauri`、`node .../vite`、
`target/debug/app`）逐一 `kill`，確認 `ps aux` 再查一次是乾淨的，然後：

```bash
cd /Users/jamesju/Documents/GitHub/AITERM-full-parity && nohup npm run tauri:dev > /tmp/aiterm-dev.log 2>&1 &
disown
```

等 20 秒後 `tail -40 /tmp/aiterm-dev.log`，確認沒有 port 衝突（`Address already
in use`）、沒有重複輪詢（`Another instance is already polling`）之類的錯誤，且
`ps aux | grep target/debug/app` 只有一個新啟動的 process。

- [ ] **Step 5: 明確告知使用者以下事項，不要自己代為判斷完成**

1. 重新測試遠端終端機分頁：即時窗格閒置時應該縮到接近提示字元一行的高度、
   帶有邊框；指令執行中應該撐高；指令完成後應該收回、變成卡片。
2. 固定字體後，若主控端的欄寬明顯比觀看端視窗寬，確認畫面出現水平捲軸、
   可以正常橫向捲動看到完整內容，而不是內容被截斷或版面跑掉。
3. 連續執行多個指令，確認每次新卡片出現都會自動捲到最底部、不需要使用者
   自己往下滑。
4. 若發現任何問題，比照這個分支一貫的作法：不要用截圖描述去猜根因，讀
   實際程式碼、寫紅燈測試證明重現、再修。
