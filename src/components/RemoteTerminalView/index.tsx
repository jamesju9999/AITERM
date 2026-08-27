import { useEffect, useRef, useState } from "react";
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
  const [phase, setPhase] = useState<Phase>({ kind: "waiting", sas });

  // `phase` 要在事件 callback 裡讀到最新值，但那些 callback 只註冊一次——
  // 用 ref 避免 stale closure（這個 repo 在 Tauri 事件監聽上踩過這個坑）。
  const phaseRef = useRef(phase);
  phaseRef.current = phase;

  // 後續一個任務會賦值成真正的字級重算函式；這裡先放 ref 讓 xterm 建立
  // 那個 effect（先寫）能呼叫到它，即使賦值它的 effect（後寫）還沒跑。
  const recomputeFontSizeRef = useRef<(() => void) | null>(null);

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
      onShareViewerGranted(connId, ({ mode, cols, rows }) => {
        // 尺寸由主控端說了算——照它給的建立，不用自己的視窗大小。
        // `mode` 為空字串代表這是後續的 resize 通知，不是初次核准。
        if (mode) setPhase({ kind: "live", mode });
        const term = termRef.current;
        if (term && cols > 0 && rows > 0) {
          term.resize?.(cols, rows);
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

    const onData = term.onData((data: string) => {
      // **唯讀時按鍵根本不送出**，不是送了被伺服器拒絕。伺服器端還有一道
      // `may_send_input`，但那是安全邊界；這一層是給使用者的即時回饋。
      const p = phaseRef.current;
      if (p.kind === "live" && p.mode === "control") {
        void shareViewerSend(connId, data);
      }
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

      <div className="aiterm-remote-terminal__screen" ref={hostRef} />
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
