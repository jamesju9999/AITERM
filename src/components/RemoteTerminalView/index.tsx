import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import {
  onShareViewerControlChanged,
  onShareViewerData,
  onShareViewerEnded,
  onShareViewerGranted,
  onShareViewerResync,
  shareViewerDisconnect,
  shareViewerSend,
} from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import { getActiveTheme, type AppTheme } from "../../lib/themes";
import { useTerminalBlocks } from "../../hooks/useTerminalBlocks";
import { WarpInput } from "../WarpInput";
import { TerminalBlockCard } from "../TerminalBlockCard";
import { addBookmark } from "../CommandBookmarks";
import { parseAiPrefix, parseAgentPrefix } from "../parseAiPrefix";
import type { Translations } from "../../lib/i18n";
import "./index.css";

interface Props {
  tabId: string;
  /** 2B-1 的觀看連線 id。所有 `share-viewer://*` 事件都掛在它上面。 */
  connId: string;
  /**
   * 這一端算出的 4 位驗證碼，**要顯示給使用者唸給對方聽**。
   *
   * 用 prop 而不是訂閱事件：它在連線建立的當下就已知（跟著
   * `shareViewerConnect` 的回傳值一起來），而這個元件要等分頁開好才掛載
   * ——用事件送必然遺失，因為發出的時候還沒有人在聽。
   */
  sas: string;
  isActive: boolean;
}

type Phase =
  | { kind: "waiting"; sas: string | null }
  | { kind: "live"; mode: string }
  | { kind: "ended"; reason: string };

export function RemoteTerminalView({ tabId, connId, sas, isActive }: Props) {
  const { t } = useLocale();
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const [termState, setTermState] = useState<Terminal | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "waiting", sas });

  // `phase` 要在事件 callback 裡讀到最新值，但那些 callback 只註冊一次——
  // 用 ref 避免 stale closure（這個 repo 在 Tauri 事件監聽上踩過這個坑）。
  const phaseRef = useRef(phase);
  phaseRef.current = phase;

  // 有控制權時才真的送出，唯讀時整個是 no-op——**唯讀時按鍵根本不送出**，
  // 不是送了被伺服器拒絕（伺服器端還有 may_send_input 這道安全邊界，這
  // 一層是給使用者的即時回饋）。用 phaseRef 讀最新值，身分只跟著 connId
  // 變，這樣可以放心把它傳給 useTerminalBlocks（見該 hook 對 write 參數
  // 的說明：它自己也用 ref 橋接，不會因為這裡身分穩定與否而有額外負擔，
  // 但兩邊都求穩比較不容易出錯）。
  const write = useCallback(
    (data: string) => {
      const p = phaseRef.current;
      if (p.kind === "live" && p.mode === "control") {
        void shareViewerSend(connId, data);
      }
    },
    [connId],
  );

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

  const { blocks, submitCommand, clearAllBlocks } = useTerminalBlocks(
    connId,
    termState,
    undefined,
    undefined,
    undefined,
    undefined,
    write,
    hostPlatform,
  );
  // `clearAllBlocks` 這個任務還用不到——下一個任務接上 Resync 才會用。
  // `noUnusedLocals` 開著，先 `void` 掉滿足型別檢查。
  void clearAllBlocks;

  const [aiUnsupported, setAiUnsupported] = useState(false);

  // WarpInput 送出的整行文字先過一次 AI 前綴檢查——跟本機分頁用同一套
  // parseAiPrefix.ts 規則，不重新猜字首。是 /ai 或 /agent 開頭就不送出、
  // 顯示提示；否則走 submitCommand（會建立分段卡片並透過 write 送出）。
  const handleWarpSubmit = useCallback(
    (cmd: string) => {
      if (parseAiPrefix(cmd) !== null || parseAgentPrefix(cmd) !== null) {
        setAiUnsupported(true);
        return;
      }
      setAiUnsupported(false);
      submitCommand(cmd);
    },
    [submitCommand],
  );

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
    if (!connId) return;
    const unlisteners: Array<() => void> = [];
    let disposed = false;

    const track = (p: Promise<() => void>) => {
      p.then((un) => {
        if (disposed) un();
        else unlisteners.push(un);
      });
    };

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

    track(
      onShareViewerData(connId, (b64) => {
        const bytes = atob(b64);
        const arr = new Uint8Array(bytes.length);
        for (let i = 0; i < bytes.length; i++) arr[i] = bytes.charCodeAt(i);
        termRef.current?.write(arr);
      }),
    );

    track(
      onShareViewerResync(connId, () => {
        // 清空再接全量重播。漏掉的位元組可能截斷 ANSI 逃脫序列，帶著壞掉
        // 的畫面繼續是不會自己好的。
        termRef.current?.clear();
      }),
    );

    track(
      onShareViewerControlChanged(connId, (mode) => {
        setPhase({ kind: "live", mode });
      }),
    );

    track(onShareViewerEnded(connId, (reason) => setPhase({ kind: "ended", reason })));

    return () => {
      disposed = true;
      for (const un of unlisteners) un();
    };
  }, [connId]);

  // xterm 的建立與銷毀。刻意跟事件訂閱分開——訂閱只依賴 connId，終端機
  // 只依賴掛載，兩者的生命週期不同。
  useEffect(() => {
    if (!hostRef.current) return;

    // 跟本機分頁（TerminalView.tsx）讀同一份設定來源，讓遠端分頁的外觀
    // 一開始就對得上，不用等使用者去改設定才同步。
    const initFontSize = parseInt(localStorage.getItem("aiterm-font-size") ?? "14", 10) || 14;
    const initFontFamily = localStorage.getItem("aiterm-font-family") ?? '"Cascadia Mono", Consolas, monospace';
    const initTheme = getActiveTheme();

    const term = new Terminal({
      fontFamily: initFontFamily,
      fontSize: initFontSize,
      lineHeight: 1.1,
      cursorBlink: true,
      theme: initTheme.xterm,
      convertEol: false,
    });
    term.open(hostRef.current);
    termRef.current = term;
    setTermState(term);

    const onData = term.onData((data: string) => {
      // Drop focus-tracking events that xterm.js emits when it loses / gains
      // focus（跟本機分頁 TerminalView.tsx 的 term.onData 同一條規則、同一個
      // 理由：PSReadLine 開了 focus tracking，逐字轉送這兩個序列會讓
      // PowerShell 把它們印成字面上的 "[O"/"[I"）。本機分頁另外還有一條
      // 「AI 面板開著時不轉送」，遠端分頁沒有 AI 面板這個概念，不複製。
      if (data === "\x1b[O" || data === "\x1b[I") return;
      write(data);
    });

    const onFontChanged = (e: Event) => {
      const { fontSize, fontFamily } = (e as CustomEvent).detail as { fontSize: number; fontFamily: string };
      term.options.fontSize = fontSize;
      term.options.fontFamily = fontFamily;
      // 字型/字級變了，同樣的 cols×rows 需要的容器空間也變了——Task 4
      // 加的 recomputeFontSize 會在這個函式存在後接手這件事。
      recomputeFontSizeRef.current?.();
    };
    window.addEventListener("aiterm:font-changed", onFontChanged);

    const onThemeChanged = (e: Event) => {
      const { theme } = (e as CustomEvent).detail as { theme: AppTheme };
      term.options.theme = theme.xterm;
    };
    window.addEventListener("aiterm:theme-changed", onThemeChanged);

    return () => {
      window.removeEventListener("aiterm:font-changed", onFontChanged);
      window.removeEventListener("aiterm:theme-changed", onThemeChanged);
      onData?.dispose?.();
      term.dispose();
      termRef.current = null;
      setTermState(null);
    };
  }, [connId]);

  // 分頁關閉時斷線。
  //
  // **StrictMode 陷阱**：dev 模式下 React 會對每個 effect 模擬「掛載→卸載→
  // 重新掛載」來抓沒清乾淨的副作用。但這條連線是在這個元件掛載**之前**就
  // 建立好的（`ConnectDialog` 裡呼叫 `shareViewerConnect`——SAS 驗證碼要在
  // 連線當下就先算出來，見該檔案的說明），所以模擬卸載時觸發的
  // `shareViewerDisconnect` 沒有對應的「重新連線」可以復原：後端把這條
  // 連線從 `ViewerManager` 移除後，`connId` 不變就不會重新連線，之後所有
  // `shareViewerSend` 都會找不到連線，觀看端看起來像唯讀（即使 UI 顯示
  // mode 是 control，畫面照樣正常，因為顯示走的是另一條事件監聽，跟這
  // 條連線是否還「活著」無關）——這正是實機測試抓到的「控制模式打字沒
  // 反應」的成因。
  //
  // 用 `setTimeout(0)` 把真正斷線延後一輪：StrictMode 的重新掛載會在
  // 這個 timer 觸發前執行，把它取消掉；真正的卸載（分頁真的被關閉）
  // 沒有後續掛載，timer 會如期觸發。
  const disconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (disconnectTimerRef.current) {
      clearTimeout(disconnectTimerRef.current);
      disconnectTimerRef.current = null;
    }
    return () => {
      if (!connId) return;
      disconnectTimerRef.current = setTimeout(() => {
        disconnectTimerRef.current = null;
        void shareViewerDisconnect(connId);
      }, 0);
    };
  }, [connId]);

  return (
    <div className="aiterm-remote-terminal" data-tab-id={tabId} data-active={isActive}>
      {phase.kind === "waiting" && (
        <div className="aiterm-remote-terminal__banner">
          <span>{t.remote_terminal_waiting_approval}</span>
          {phase.sas && (
            <>
              {/* 觀看端**必須**顯示自己算出的碼——那是要唸給對方聽的。
                  主控端相反：那邊絕不顯示自己的碼，否則使用者會照抄而不
                  問對方，人工核對變成自欺。兩邊不對稱是刻意的。 */}
              <strong className="aiterm-remote-terminal__sas">{phase.sas}</strong>
              <span className="aiterm-remote-terminal__hint">{t.remote_terminal_your_code}</span>
            </>
          )}
        </div>
      )}

      {phase.kind === "live" && phase.mode === "read_only" && (
        <div className="aiterm-remote-terminal__banner aiterm-remote-terminal__banner--readonly">
          {t.remote_terminal_read_only}
        </div>
      )}

      {phase.kind === "ended" && (
        <div className="aiterm-remote-terminal__banner aiterm-remote-terminal__banner--ended">
          {endReasonText(t, phase.reason)}
        </div>
      )}

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

      {aiUnsupported && (
        <div className="aiterm-remote-terminal__ai-unsupported">{t.remote_terminal_ai_unsupported}</div>
      )}

      <WarpInput
        onSubmit={handleWarpSubmit}
        disabled={!(phase.kind === "live" && phase.mode === "control")}
      />
    </div>
  );
}

/**
 * 把後端的 `EndReason` 字串轉成一句人話。
 *
 * spec 要求**不能有「未知錯誤」**。認不得的 reason（例如對方是更新版）
 * 也要給一句話，而不是把原始字串丟到畫面上。
 *
 * `t` 是具名的 `Translations` 型別，不是 `Record<string, string>`——用
 * 變數當 key 去索引具名型別，TypeScript 會擋，所以這裡先轉成
 * `Record<string, string>` 再查。這是刻意的 escape hatch，範圍限縮在這
 * 一個函式裡。
 */
function endReasonText(t: Translations, reason: string): string {
  const key = `remote_terminal_ended_${reason}`;
  const table = t as unknown as Record<string, string>;
  return table[key] ?? t.remote_terminal_ended_session_closed;
}

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
