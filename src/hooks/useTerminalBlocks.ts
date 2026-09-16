import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal, type IMarker } from "@xterm/xterm";
import { writePty } from "../ipc/pty";
import { parseAnsiToRenderedLines, readRenderedLines, findContentEndRow, type RenderedLine } from "../lib/ansiBlockParser";
import type { GitBlockInfo } from "../ipc/vcs";

export interface TerminalBlock {
  id: string;
  command: string;
  status: "running" | "completed" | "failed";
  exitCode?: number;
  startTime: number;
  endTime?: number;
  cwd?: string;
  rawOutput: string;
  renderedLines?: RenderedLine[];
  gitInfo?: GitBlockInfo | null;
}

export interface UseTerminalBlocksResult {
  blocks: TerminalBlock[];
  submitCommand: (cmd: string, onComplete?: (block: TerminalBlock) => void) => void;
  beginTrackedBlock: (cmd: string) => void;
  appendOutput: (chunk: string) => void;
  setBlockGitInfo: (id: string, info: GitBlockInfo | null) => void;
  isAlternateBuffer: boolean;
  /** true 代表遠端目前處於「要逐鍵收原始按鍵」的協定模式（Kitty keyboard
   *  protocol 的 push/pop，`ESC[>Ps u` / `ESC[<u`）——不是所有互動選單都會
   *  切 alternate screen buffer，這個訊號補上那個縫。見設計文件
   *  docs/superpowers/specs/2026-09-16-interactive-prompt-live-expand-design.md。 */
  isRawKeyboardModeActive: boolean;
  termInstance: Terminal | null;
  /** 強制把一個 running 中的區塊結案（例如卡在 heredoc 的中斷）。
   *  會呼叫該區塊等待中的 onComplete callback——見 finalizeBlock 內部實作。 */
  finalizeBlock: (blockId: string, exitCode: number, opts?: { clearOnParsed?: boolean }) => void;
  /** 清空整個分段卡片歷史。原本只在內部處理 `clear`/`cls` 指令時用；
   *  遠端分頁在收到 `Resync`（漏位元組、全量重播）時也要呼叫這個——
   *  漏掉的位元組可能連帶讓卡片內容跟畫面對不上，這跟本機分頁執行
   *  `clear`/`cls` 時「畫面跟卡片一起清空」是同一個邏輯。 */
  clearAllBlocks: () => void;
}

// `clear` is the Unix/PowerShell screen-clear command; `cls` is cmd.exe's (and
// also a built-in PowerShell alias for Clear-Host, alongside `clear`). Matched
// case-insensitively since Windows commands aren't case-sensitive.
function isClearCommand(cmd: string): boolean {
  const trimmed = cmd.trim().toLowerCase();
  return trimmed === "clear" || trimmed === "cls";
}

/**
 * 從一個絕對緩衝區座標（OSC 133 B 標記記錄的「輸入從這裡開始」位置）到目前
 * 遊標所在行，把畫面上的文字截出來當作還原出的指令文字。只在「沒有本機
 * 追蹤區塊」時才會被呼叫——見設計文件
 * docs/superpowers/specs/2026-08-27-remote-command-text-recovery-design.md
 * 的「recoverUntrackedCommand() 演算法」一節。
 */
function recoverUntrackedCommand(
  term: Terminal,
  promptEnd: { row: number; col: number } | null,
): string | null {
  if (!promptEnd) return null;
  const { row: startRow, col: startCol } = promptEnd;
  // OSC C 觸發時，遊標已經因為 Enter 換行到新的一行，所以往上一行才是輸入
  // 內容實際結束的地方。
  const endRow = term.buffer.active.cursorY + term.buffer.active.baseY - 1;
  if (endRow < startRow) return null;

  let fullLine = "";
  for (let row = startRow; row <= endRow; row++) {
    const line = term.buffer.active.getLine(row);
    if (!line) return null;
    // A row after the first must be a genuine auto-wrap continuation of the
    // previous row (isWrapped) — a real newline in between (e.g. a remote
    // viewer's multi-line paste landing before the actual submitting Enter)
    // means this isn't a single logical input line anymore, and gluing the
    // rows together would silently fabricate wrong command text that LOOKS
    // like a successful recovery. Bail out to the safe fallback instead.
    if (row > startRow && !line.isWrapped) return null;
    fullLine += row === startRow ? line.translateToString(true, startCol) : line.translateToString(true);
  }
  const trimmed = fullLine.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function useTerminalBlocks(
  sessionId: string,
  term: Terminal | null,
  cwdRef?: React.RefObject<string>,
  onLiveClear?: () => void,
  /** 每次有指令跑完就呼叫，帶上它的 exit code。給側邊欄提示點用。
   *  必須是穩定的參考（useCallback 空依賴或 ref 橋接）——它進了下面
   *  OSC handler effect 的依賴陣列，每次換身分都會重新註冊 handler。 */
  onCommandSettled?: (exitCode: number) => void,
  /** 每次開始追蹤一個新指令就呼叫，帶上指令文字。給「偵測使用者跑了什麼」用。
   *  必須是穩定的參考（useCallback 空依賴或 ref 橋接）——它進了 submitCommand
   *  與 beginTrackedBlock 的依賴陣列，每次換身分都會讓兩者的識別跟著變。 */
  onCommandStarted?: (cmd: string) => void,
  /** 指令怎麼寫出去。預設包一層 `writePty(sessionId, data)`，跟改動前的
   *  行為完全一樣。遠端分頁傳 `(data) => shareViewerSend(connId, data)`。
   *
   *  **不要把這個參數本身放進任何 useEffect/useCallback 的依賴陣列**：
   *  呼叫端沒有明確傳值時會落到這個預設值運算式，而預設參數是每次呼叫
   *  都重新求值的——本機分頁因此每次 render 都會拿到一個全新的函式參考。
   *  下面用 `writeRef` 橋接解決，內部一律呼叫 `writeRef.current(...)`。 */
  write: (data: string) => void = (data) => writePty(sessionId, data),
  /** 主控端平台，只影響 Windows ConPTY 的 Ctrl+L 清畫面同步邏輯。預設讀
   *  `navigator.platform`，跟改動前行為一樣；遠端分頁傳 `Granted` 訊息裡
   *  的 `host_os`。這是字串值，可以放心放進依賴陣列（不像 `write` 是函式
   *  參考，同樣的字串值不會觸發 React 重新執行 effect）。 */
  hostPlatform: "windows" | "other" = navigator.platform.toLowerCase().startsWith("win") ? "windows" : "other",
): UseTerminalBlocksResult {
  const [blocks, setBlocks] = useState<TerminalBlock[]>([]);
  const [isAlternateBuffer, setIsAlternateBuffer] = useState(false);
  const [isRawKeyboardModeActive, setIsRawKeyboardModeActive] = useState(false);
  // Kitty keyboard protocol 的 push/pop 是可疊加的堆疊語意，用深度計數器
  // 而不是布林值，理由見上面 interface 欄位的註解連結的設計文件。
  const rawKeyboardDepthRef = useRef(0);

  const writeRef = useRef(write);
  writeRef.current = write;

  const blocksRef = useRef<TerminalBlock[]>([]);
  const completionCallbacksRef = useRef<Map<string, (block: TerminalBlock) => void>>(new Map());
  // OSC 133 B 標記記錄的「輸入從這裡開始」絕對座標，給 recoverUntrackedCommand
  // 用——只在遠端指令（沒有本機追蹤區塊）時才會被讀取，見該函式的文件註解。
  const promptEndRef = useRef<{ row: number; col: number } | null>(null);
  // 一個 running 中區塊開始時（OSC 133 C 當下、或 submitCommand/
  // beginTrackedBlock 建立區塊當下）游標所在行＝這個指令輸出的第一行。
  // 原本只有 Windows 才登記（ConPTY 用絕對座標畫在自己固定大小的畫面上，
  // 拿去另一個終端機從左上角重播會整段互相覆蓋，卡片內容因此要從這一行
  // 起直接讀畫面緩衝區），現在所有平台都登記——見設計文件
  // docs/superpowers/specs/2026-09-16-live-block-rendering-design.md：
  // running 中的區塊現在也要持續讀取畫面緩衝區更新 renderedLines
  // （見下面的 scheduleLiveRender），不是只有結束時讀一次。
  const outputStartRef = useRef<{ blockId: string; marker: IMarker } | null>(null);
  // scheduleLiveRender 節流：同一個節流週期內收到多個 chunk，只在最後一次
  // 真正重新掃描緩衝區算 renderedLines，不會累積成「chunk 數 × 目前已輸出
  // 列數」的計算量。用 setTimeout 不用 requestAnimationFrame：這裡只是
  // 節流一段純資料處理（沒有東西要畫），沒有理由跟瀏覽器的畫面更新週期
  // 綁在一起，setTimeout 語意更直接。
  const liveRenderRafRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  /**
   * 實機測試抓到的第二個 bug（跟 appendOutput 的 race 是不同根因）：
   * finalizeBlock 的 term.clear() 要等非同步的 parseAnsiToRenderedLines
   * 解析完才觸發（見下方 finalizeBlock 的文件註解，這是刻意的設計，避免
   * 「畫面先變空白、卡片才出現」的閃爍）。如果這個延遲夠久，久到下一個
   * 指令（可能是遠端觀看者送的）自己的 OSC 133 B 已經先被記錄進
   * promptEndRef，這次延遲的 clear() 一旦真的執行，就會讓那個座標對不
   * 上——但這裡不需要因此放棄「清畫面時機保持不變」這個對本機終端機
   * 已經運作良好的設計。
   *
   * 讀 xterm.js 原始碼（node_modules/@xterm/xterm/src/browser/Terminal.ts
   * 的 clear()）確認它實際上不是「整個清空重來」：它把「遊標目前所在的
   * 那一行」搬去當新的第 0 行，其餘全部丟棄。只要 clear() 觸發的當下，
   * 遊標還停在 promptEndRef 記錄的那一行（也就是使用者還沒按下 Enter、
   * 那一行的內容還原封不動），這一行的內容就會被完整保留、只是行號
   * 變成 0——欄位（col）完全不受影響。這裡就是在 clear() 發生的同時，
   * 讓 promptEndRef 的行號跟著一起「搬家」，而不是讓它整個作廢。
   *
   * 「遊標還停在 promptEndRef 記錄的那一行」這個前提**必須先檢查、不能
   * 假設一定成立**：如果使用者輸入的指令已經因為欄寬不夠而換行（游標
   * 現在停在接續行，不是 B 當初記錄的那一行），clear() 保留下來的是
   * 接續行的內容，跟 promptEndRef.col 對不上——這時候若還是無條件把
   * row 搬成 0，recoverUntrackedCommand 會從錯誤的一行切出內容，得到
   * 一個看似成功、實際上是錯的還原結果，比直接還原失敗（不建立區塊）
   * 還糟（還原失敗不會生出錯誤資料）。
   * 所以要在呼叫 clear() 之前，先讀一次目前遊標的絕對行號，只有在它
   * 剛好等於 promptEndRef 記錄的行號時，才代表這個前提成立、可以安心
   * 搬遷；不成立就維持原樣，讓既有的 endRow < startRow 防呆邏輯自然
   * 接手判斷失敗。
   */
  const clearAndRebasePromptEnd = useCallback((t: Terminal) => {
    const cursorRowBeforeClear = t.buffer.active.cursorY + t.buffer.active.baseY;
    t.clear();
    if (promptEndRef.current && promptEndRef.current.row === cursorRowBeforeClear) {
      promptEndRef.current = { row: 0, col: promptEndRef.current.col };
    }
  }, []);

  const updateLatestBlock = useCallback((updater: (b: TerminalBlock) => TerminalBlock) => {
    const prev = blocksRef.current;
    if (prev.length === 0) return;
    const latest = prev[prev.length - 1];
    const updated = prev.map((b) => (b.id === latest.id ? updater(b) : b));
    blocksRef.current = updated;
    setBlocks(updated);
  }, []);

  /**
   * 執行中的區塊現在跟已完成的卡片用同一套 `readRenderedLines` 顯示（見
   * 設計文件 2026-09-16-live-block-rendering-design.md），不再用另一個
   * 固定大小、需要對齊的即時窗格——這個函式負責把「目前畫面緩衝區算出來
   * 的最終視覺狀態」持續寫回這個 block 的 `renderedLines`。
   *
   * 用 setTimeout 節流（不用 requestAnimationFrame——見 liveRenderRafRef
   * 宣告處的說明）：`readRenderedLines` 是線性掃描（從 marker 那一行掃到
   * 目前游標所在列），不是遞增式的，狂送輸出的指令若每個 chunk 都重新掃
   * 一次，成本會隨輸出量變差。同一個節流週期內收到多個 chunk，只在最後
   * 一次真正掃描一次。
   */
  const scheduleLiveRender = useCallback(
    (blockId: string) => {
      if (liveRenderRafRef.current !== null) return;
      liveRenderRafRef.current = setTimeout(() => {
        liveRenderRafRef.current = null;
        const start = outputStartRef.current;
        if (!term || !start || start.blockId !== blockId) return;
        const latest = blocksRef.current[blocksRef.current.length - 1];
        if (!latest || latest.id !== blockId || latest.status !== "running") return;

        const buf = term.buffer.active;
        // 標記被 scrollback 修剪掉時 line 會是 -1——代表輸出比 scrollback
        // 還長，剩下的每一列都屬於這個指令，從第 0 列讀起。
        //
        // 不用游標目前所在列當 endRow：互動式選單常見的畫法是整個選單
        // （含裝飾用的提示文字，例如 claude CLI 信任提示最下面那行
        // 「Press Ctrl-C again to exit」）先一次寫完，再把游標移回選單中
        // 「目前選到的那一項」顯示反白——這些位元組通常在同一批 PTY
        // chunk、同一個節流週期內一起處理完，我們只讀得到「已經移回去」
        // 之後的最終游標位置，沒有機會在游標抵達最深列的當下拍一張快照
        // （試過用一顆 ref 記住「曾經到過的最深列」，這裡是它 across
        // 多個節流週期才有用，但實機測試證實這個情境整批內容在單一週期
        // 內就處理完了，那個 ref 永遠只看得到搬回去之後的狀態，完全沒
        // 機會累積到那個峰值）。改成從游標往下逐列掃描（findContentEndRow），
        // 只要還連續有內容就往下擴張，一遇到空白列就停手，上限是一整個
        // 螢幕高度（互動選單很少會比一個畫面還高）——這樣游標之後「同一批
        // 一次畫完」的內容不會被切掉，但也不會像先前無條件多讀一整個
        // term.rows 那樣，隔著一段空白硬讀到緩衝區更下面、屬於這個分頁
        // 更早一個指令、從未被清空過的殘留內容（實機抓到的 bug：`ls -la`
        // 卡片尾端混進同一個分頁更早跑過的 `ifconfig` 殘留行，中間隔著
        // 一大段空白）。
        const cursorRow = buf.baseY + buf.cursorY;
        const endRow = findContentEndRow(buf, cursorRow, start.marker.line + term.rows, term.cols);
        const renderedLines = readRenderedLines(buf, Math.max(0, start.marker.line), endRow, term.cols);

        const withLines = blocksRef.current.map((b) => (b.id === blockId ? { ...b, renderedLines } : b));
        blocksRef.current = withLines;
        setBlocks(withLines);
      }, 16);
    },
    [term],
  );

  const appendOutput = useCallback(
    (chunk: string) => {
      updateLatestBlock((b) => (b.status === "running" ? { ...b, rawOutput: b.rawOutput + chunk } : b));
      const latest = blocksRef.current[blocksRef.current.length - 1];
      if (latest?.status === "running") scheduleLiveRender(latest.id);
    },
    [updateLatestBlock, scheduleLiveRender],
  );

  const setBlockGitInfo = useCallback((id: string, info: GitBlockInfo | null) => {
    const prev = blocksRef.current;
    const updated = prev.map((b) => (b.id === id ? { ...b, gitInfo: info } : b));
    blocksRef.current = updated;
    setBlocks(updated);
  }, []);

  // Wipes the whole block history — used when the user runs `clear`, which in a
  // real terminal resets everything visible, not just whatever the live viewport
  // happens to be showing. Any block still "running" at this point is simply
  // dropped: appendOutput/finalizeBlock already no-op safely against an empty
  // blocksRef (see their own `prev.length === 0` guards), so no crash risk.
  const clearAllBlocks = useCallback(() => {
    blocksRef.current = [];
    setBlocks([]);
    outputStartRef.current?.marker.dispose();
    outputStartRef.current = null;
    if (liveRenderRafRef.current !== null) {
      clearTimeout(liveRenderRafRef.current);
      liveRenderRafRef.current = null;
    }
    rawKeyboardDepthRef.current = 0;
    setIsRawKeyboardModeActive(false);
  }, []);

  /**
   * Marks a still-running block as completed/failed, freezes its rawOutput,
   * and does one final synchronous re-render of its `renderedLines` before
   * firing its onComplete callback. Shared by the normal OSC 133 D path and
   * the defensive "orphaned block" path in submitCommand below, so a block
   * can never be finalized more than once and its callback can never be left
   * dangling regardless of which path finalizes it.
   *
   * `renderedLines` has already been kept live-updated by scheduleLiveRender
   * as output streamed in (see its doc comment), but that update is
   * rAF-throttled — there could be a chunk that arrived just before OSC 133 D
   * whose scheduled update hasn't fired yet. This does one last synchronous
   * read (not scheduled) so the final card never misses trailing output, and
   * cancels the now-redundant pending rAF.
   *
   * `opts.clearOnParsed` clears the live terminal now that the card has this
   * command's final content (the original, Mac/Linux behavior — avoids a
   * blank-screen flash between "raw output disappears" and "card appears").
   * The OSC 133 D path skips this on Windows, where it instead clears
   * synchronously-deferred and force-repaints — see the registerOscHandler
   * callback below for why.
   */
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

      if (liveRenderRafRef.current !== null) {
        clearTimeout(liveRenderRafRef.current);
        liveRenderRafRef.current = null;
      }

      const endTime = Date.now();
      const frozenOutput = target.rawOutput;
      const cols = term?.cols ?? 80;

      // 標記被 scrollback 修剪掉時 line 會是 -1——代表輸出比 scrollback 還長，
      // 剩下的每一列都屬於這個指令，從第 0 列讀起。沒有登記過 marker（理論上
      // 不會發生，除非區塊建立當下 term 是 null）才退回重新解析凍結文字。
      const start = outputStartRef.current;
      let renderedLines: RenderedLine[] | null = null;
      if (start?.blockId === blockId && term) {
        const buf = term.buffer.active;
        // 跟 scheduleLiveRender 同一個理由、同一套算法（見該處的說明）：
        // 結案當下游標可能還停在互動選單中段，不是內容實際畫到的最深列。
        const cursorRow = buf.baseY + buf.cursorY;
        const endRow = findContentEndRow(buf, cursorRow, start.marker.line + term.rows, term.cols);
        renderedLines = readRenderedLines(buf, Math.max(0, start.marker.line), endRow, cols);
        start.marker.dispose();
        outputStartRef.current = null;
      }

      const finalized: TerminalBlock = {
        ...target,
        status: exitCode === 0 ? "completed" : "failed",
        exitCode,
        endTime,
        ...(renderedLines ? { renderedLines } : null),
      };
      const updated = prev.map((b) => (b.id === blockId ? finalized : b));
      blocksRef.current = updated;
      setBlocks(updated);

      const settle = (finalBlock: TerminalBlock) => {
        const cb = completionCallbacksRef.current.get(blockId);
        if (cb) {
          completionCallbacksRef.current.delete(blockId);
          setTimeout(() => cb(finalBlock), 50);
        }
      };

      if (renderedLines) {
        // Marker path: renderedLines were computed synchronously above, so
        // it's safe to clear/rebase right away — no async gap during which
        // a new prompt cycle's OSC 133 B could record coordinates against
        // still-unlcleared content.
        if (opts?.clearOnParsed && term) clearAndRebasePromptEnd(term);
        settle(finalized);
      } else {
        parseAnsiToRenderedLines(frozenOutput, cols).then((lines) => {
          const withLines = blocksRef.current.map((b) => (b.id === blockId ? { ...b, renderedLines: lines } : b));
          blocksRef.current = withLines;
          setBlocks(withLines);
          // Fallback path: only clear/rebase once the async parse has
          // actually captured the frozen output — clearing earlier would
          // race a new prompt's B marker (see TerminalView.staleClearRace.test.tsx).
          if (opts?.clearOnParsed && term) clearAndRebasePromptEnd(term);
          settle(withLines.find((b) => b.id === blockId)!);
        });
      }
    },
    [term, clearAndRebasePromptEnd],
  );

  /**
   * Starts tracking a block for a command that was typed directly into the
   * live terminal (bypassing WarpInput's submitCommand — WarpInput isn't the
   * only way to type into a real terminal). Unlike submitCommand, this does
   * NOT write anything to the PTY: the caller (TerminalView's onData handler,
   * or the OSC 133 B/C recovery path below for remote-viewer-issued commands)
   * has already streamed the keystrokes to the PTY, so writing here again
   * would duplicate/corrupt input. This only does the block-bookkeeping half
   * of submitCommand.
   *
   * 搬到這裡（原本在 submitCommand 之後、檔案偏下方）是因為下面的 OSC 133
   * effect 需要直接呼叫它——effect 的 closure 抓的是變數本身，函式定義必須
   * 出現在它前面。
   */
  const beginTrackedBlock = useCallback(
    (cmd: string) => {
      if (!sessionId) return;

      onCommandStarted?.(cmd);

      if (isClearCommand(cmd)) {
        // Same reasoning as submitCommand's `clear`/`cls` handling — wipe the
        // whole block history instead of tracking a card for it. The keystrokes
        // (including the trailing Enter) are already streaming to the PTY via
        // onData, so there's nothing to write here.
        clearAllBlocks();
        return;
      }

      const prevBlocks = blocksRef.current;
      const prevLatest = prevBlocks[prevBlocks.length - 1];
      if (prevLatest?.status === "running") {
        // Already tracking a block — most likely this Enter press belongs to
        // a submitCommand-initiated command whose OSC 133 D hasn't fired yet.
        // Don't create a second, competing block.
        return;
      }

      const newBlock: TerminalBlock = {
        id: Math.random().toString(36).substring(2, 15) + Date.now().toString(36),
        command: cmd,
        status: "running",
        startTime: Date.now(),
        cwd: cwdRef?.current,
        rawOutput: "",
      };

      const updated = [...blocksRef.current, newBlock];
      blocksRef.current = updated;
      setBlocks(updated);
    },
    [sessionId, cwdRef, clearAllBlocks, onCommandStarted],
  );

  useEffect(() => {
    if (!term) return;

    const onBufferChange = () => {
      setIsAlternateBuffer(term.buffer.active.type === "alternate");
    };
    const disposeBuffer = term.buffer.onBufferChange(onBufferChange);
    // 註冊當下就同步一次，不要只等事件。
    //
    // 這個 effect 的 deps 含三個 callback（finalizeBlock / onLiveClear /
    // onCommandSettled），任一個識別性改變就會重跑：舊監聽 dispose、新監聽
    // 註冊——中間沒有人重讀「現在是不是 alternate」。於是若程式已經切進
    // alt-screen 之後才發生重跑，狀態會永遠停在 false，畫面顯示會跟著算錯
    // （isAlternateBuffer 消費端依賴這個狀態決定要不要撐滿可用高度）。
    //
    // 冪等：setState 同值不會觸發重繪。
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
      if (data === "B") {
        // Prompt text has just finished being drawn (this marker is embedded
        // at the tail of the shell's PS1/prompt output itself — see
        // src-tauri/src/pty/shell.rs — so it's guaranteed to arrive AFTER the
        // visible prompt characters, unlike A which fires from a hook BEFORE
        // the prompt is drawn). Record exactly where input begins so
        // recoverUntrackedCommand can slice from here.
        promptEndRef.current = {
          row: term.buffer.active.cursorY + term.buffer.active.baseY,
          col: term.buffer.active.cursorX,
        };
        return true;
      } else if (data === "C") {
        // Command start — usually a no-op, since the block was already
        // created synchronously by submitCommand (WarpInput) or
        // beginTrackedBlock (typed directly into the live terminal), both of
        // which run well before this async shell-emitted event round-trips
        // back to the frontend. But if nothing is tracked as "running" at
        // this point, the command didn't come through either of those two
        // paths — it was written to the PTY some other way (e.g. a remote
        // viewer with control access). Try to recover the actual typed text
        // from the screen content between the last B marker and here; only
        // fall back to the lighter-weight boundary signal if that fails (see
        // recoverUntrackedCommand's doc comment for when/why it can fail).
        const prev = blocksRef.current;
        const latest = prev[prev.length - 1];
        if (!latest || latest.status !== "running") {
          const recovered = recoverUntrackedCommand(term, promptEndRef.current);
          if (recovered !== null) beginTrackedBlock(recovered);
        }
        // 這個指令輸出的第一行＝現在游標所在行——所有平台都要登記（原本
        // 只有 Windows，見 outputStartRef 宣告處的說明）：running 中的
        // 區塊現在要靠這個 marker 持續讀取畫面緩衝區更新 renderedLines，
        // 不是只有結束時讀一次。
        const running = blocksRef.current[blocksRef.current.length - 1];
        if (running?.status === "running" && outputStartRef.current?.blockId !== running.id) {
          const marker = term.registerMarker(0);
          if (marker) {
            outputStartRef.current?.marker.dispose();
            outputStartRef.current = { blockId: running.id, marker };
          }
        }
        return true;
      } else if (data.startsWith("D")) {
        const parts = data.split(";");
        // parts.length > 1 也拿來判斷「這次的 D 到底有沒有帶 exit code」，
        // 給下面的 onCommandSettled 用——這裡沿用既有 parse 出來的 exitCode
        // 本身完全不動，只是額外記住它是不是真的被送出來的。
        const hasExitCode = parts.length > 1;
        const exitCode = hasExitCode ? parseInt(parts[1], 10) : 0;

        const prev = blocksRef.current;
        if (prev.length === 0) return true;
        const latest = prev[prev.length - 1];
        if (latest.status !== "running") return true;

        // Windows-only: deliberately does NOT clear the xterm buffer.
        //
        // Root-caused from real-machine [cursor-diag] logging: term.clear()
        // moves the cursor's line to row 0 and discards the rest, so xterm
        // then believes the prompt sits on row 1 — while ConPTY, never told
        // any of this happened, still has it on row 12 of its own fixed-size
        // screen. PSReadLine repaints the input line with ABSOLUTE cursor
        // positioning (ESC[12;62H) on every keystroke, so once the two
        // disagree, every repaint lands on a row the user isn't looking at
        // and the visible prompt line freezes — reported as "typing a command
        // a second time gets stuck after the first character". The same
        // client-side-only illusion is why a later window resize could make
        // ConPTY resurface old output: it genuinely still has it.
        //
        // zsh/bash repaint with relative moves (CR + backspace), so they are
        // immune and keep the original clearing behavior below. On Windows
        // the buffer now stays in lockstep with ConPTY; old output never gets
        // shown twice because each card's `renderedLines` is scoped to its
        // own marker-to-cursor range (see scheduleLiveRender/finalizeBlock),
        // not the whole buffer — no cooperation from ConPTY needed.
        const isWindows = hostPlatform === "windows";
        if (isWindows) {
          setTimeout(() => {
            term?.scrollToBottom();
            onLiveClear?.();
          }, 0);
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode);
        } else {
          finalizeBlock(latest.id, isNaN(exitCode) ? 0 : exitCode, { clearOnParsed: true });
        }

        // 兩條分支（Windows/ConPTY 與其他平台）的差別只在畫面清除時機，
        // 對「指令結束了、結果是什麼」沒有影響，所以放在合流之後呼叫一次。
        //
        // 但只有真的帶了 exit code 才通知側邊欄。cmd.exe 的 PROMPT（見
        // src-tauri/src/pty/shell.rs）只送出裸的 "D"，不像 PowerShell/zsh/bash
        // 那樣附上 exit code——這種情況下無法分辨指令是成功還是失敗，若照舊
        // 假設成 0 會把失敗的指令顯示成綠色的 "done"，是誤報。沒有 exit code
        // 就什麼提示點都不設，符合「寧可漏報，不可誤報」的原則。
        if (hasExitCode) {
          onCommandSettled?.(isNaN(exitCode) ? 0 : exitCode);
        }

        return true;
      }
      return false;
    });

    return () => {
      disposeBuffer.dispose();
      disposeOsc.dispose();
      disposeRawKbPush.dispose();
      disposeRawKbPop.dispose();
    };
    // `write` 故意不在這裡——它透過 writeRef 讀取，不需要讓這個 effect
    // 跟著它的身分重新註冊（本機分頁沒傳 write 時，每次 render 呼叫端拿到
    // 的都是函式簽名裡那個預設值運算式產生的全新參考，放進依賴陣列會讓
    // 這個 effect 每次 render 都 dispose+重新註冊）。`hostPlatform` 是字串，
    // 沒有這個問題，放心加進來。
  }, [term, finalizeBlock, beginTrackedBlock, clearAndRebasePromptEnd, onLiveClear, onCommandSettled, hostPlatform]);

  const submitCommand = useCallback(
    (cmd: string, onComplete?: (block: TerminalBlock) => void) => {
      if (!term || !sessionId) return;

      onCommandStarted?.(cmd);

      // On Windows conpty: \x15 echoes as visible "^U", and \x1b gets merged with
      // the first char of the command as an Alt+key (e.g. \x1b + "d" = Alt+D which
      // deletes a word, dropping the "d").  WarpInput owns all keyboard input so the
      // PTY line is always empty — no clear sequence needed on Windows.
      // On macOS/Linux, \x15 (Ctrl+U) clears bash/zsh input silently.
      const isWindows = hostPlatform === "windows";
      const clearSeq = isWindows ? "" : "\x15";

      if (isClearCommand(cmd)) {
        // `clear`/`cls` wipes the whole block history, not just the live viewport —
        // matches what a real terminal's clear does. Still forward the command
        // to the shell (keeps shell-side history/state in sync) but don't track
        // a block for it — there's nothing meaningful to show in a card for it.
        clearAllBlocks();
        writeRef.current(clearSeq + cmd + "\r");
        return;
      }

      // If the previous block is still "running", its OSC 133 D never fired
      // (caller raced ahead, or the shell doesn't reliably emit exactly one D
      // per command). Defensively finalize it as failed/interrupted (exitCode
      // -1 is a sentinel, not a real process exit code) so it never lingers
      // forever and its onComplete callback — which the agent loop awaits —
      // is never silently dropped, leaking the completionCallbacksRef entry
      // and hanging the caller.
      const prevBlocks = blocksRef.current;
      if (prevBlocks.length > 0) {
        const prevLatest = prevBlocks[prevBlocks.length - 1];
        if (prevLatest.status === "running") {
          finalizeBlock(prevLatest.id, -1);
        }
      }

      const newBlock: TerminalBlock = {
        id: Math.random().toString(36).substring(2, 15) + Date.now().toString(36),
        command: cmd,
        status: "running",
        startTime: Date.now(),
        cwd: cwdRef?.current,
        rawOutput: "",
      };

      if (onComplete) {
        completionCallbacksRef.current.set(newBlock.id, onComplete);
      }

      const updated = [...blocksRef.current, newBlock];
      blocksRef.current = updated;
      setBlocks(updated);

      // Clear the current line before sending the command (see isWindows/clearSeq above).
      writeRef.current(clearSeq + cmd + "\r");
    },
    [sessionId, term, cwdRef, finalizeBlock, clearAllBlocks, onCommandStarted, hostPlatform],
  );

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
}
