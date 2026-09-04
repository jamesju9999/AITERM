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
import { CommandBookmarksPicker, addBookmark } from "../CommandBookmarks";
import { parseAiPrefix, parseAgentPrefix } from "../parseAiPrefix";
import { LinkIcon, SparklesIcon } from "../Icons";
import type { Translations } from "../../lib/i18n";
import type { RemoteCtx } from "../../ipc/ai";
import { getConfig } from "../../ipc/config";
import { listProviders } from "../../ipc/provider";
import { ProviderPalette } from "../ProviderPalette";
import { QuotaBadge } from "../QuotaBadge";
import { useProviderQuota } from "../../hooks/useProviderQuota";
import { RemoteAiPanel, type RemoteAiPanelHandle } from "./RemoteAiPanel";
import "../TerminalView.css";
import "./index.css";

const MIN_LIVE_ROWS = 3;
const MAX_LIVE_ROWS = 16;

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
  /**
   * 連線當下輸入的「host:port」字串，跟視窗標題「遠端終端機：10.10.41.1:
   * 50281」同一份資料（`ConnectDialog.onConnected` 回傳的 `hostLabel`，
   * 已經存在 `tab.remoteHostLabel`）。工具列的位址文字用它。
   *
   * 選填、預設空字串，不是必填：這個 prop 是這次新增的，選填可以讓既有
   * 測試呼叫端不用全部跟著改。
   */
  hostLabel?: string;
  /**
   * 使用者點了工具列的「連線」按鈕。由 `TerminalApp.tsx` 提供：開啟
   * `ConnectDialog`，並記住是「這個分頁」要求重新連線——連線成功後
   * `TerminalApp.tsx` 會更新這個分頁的 remoteConnId/remoteSas/
   * remoteHostLabel，不會開新分頁（見 Task 3）。
   *
   * 必填、沒有預設值：這顆按鈕點了沒反應會很奇怪，沒有有意義的
   * no-op 預設可以退回。
   */
  onConnectClick: () => void;
}

type Phase =
  | { kind: "waiting"; sas: string | null }
  | { kind: "live"; mode: string }
  | { kind: "ended"; reason: string };

export function RemoteTerminalView({ tabId, connId, sas, isActive, hostLabel = "", onConnectClick }: Props) {
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

  // Granted 事件的 hostOs 空字串代表「這是後續 resize 通知，不是初次
  // 授權」——只有非空字串才更新，讓 hostPlatform 維持第一次拿到的值。
  const [hostPlatform, setHostPlatform] = useState<"windows" | "other">("other");

  // 主控端最後一次告知的實際列數——全螢幕程式（isAlternateBuffer）使用中
  // 的即時窗格高度要靠它算，見下面 altBufferHeightPx 的說明。用 state
  // 而不是直接讀 term.rows：單純的 resize（列數變了但 isAlternateBuffer
  // 本身沒變、也沒有其他 state 跟著變）不會讓這個元件重新 render，
  // term.rows 讀到的會是上一輪 render 當下的舊值，畫面因此不會跟著更新。
  const [hostRows, setHostRows] = useState(MAX_LIVE_ROWS);

  // 見下方 liveTopRows 的說明——OSC 133 B 回報提示字元位置時要用到，但
  // 真正的實作宣告在後面，所以跟本檔其他同類情況一樣先用 ref 佔位。
  const syncLiveTopRef = useRef<(() => void) | null>(null);

  // 「有指令在跑，但不是這一端發起的」的保底訊號——同一顆佔位 ref 手法，
  // 真正的實作要等 setLiveRows 宣告完才能賦值（見下方賦值處）。
  const untrackedCommandBoundaryRef = useRef<((kind: "start" | "end") => void) | null>(null);
  const handleUntrackedCommandBoundary = useCallback((kind: "start" | "end") => {
    untrackedCommandBoundaryRef.current?.(kind);
  }, []);
  const promptAbsRowRef = useRef<number | null>(null);
  const handlePromptStart = useCallback((absoluteRow: number) => {
    promptAbsRowRef.current = absoluteRow;
    syncLiveTopRef.current?.();
  }, []);

  const { blocks, isAlternateBuffer, submitCommand, appendOutput, clearAllBlocks } = useTerminalBlocks(
    connId,
    termState,
    undefined,
    undefined,
    undefined,
    undefined,
    write,
    hostPlatform,
    handleUntrackedCommandBoundary,
    handlePromptStart,
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

  // 已連線時間從進入 live 的那一刻開始算，用 `=== null` 當 guard 只寫入
  // 一次——`phase.kind` 進了 "live" 之後，後續的控制權變更
  // （onShareViewerControlChanged）會用不同的 mode 再次呼叫
  // setPhase({kind:"live", ...})，但 `phase.kind` 這個依賴值本身沒變，
  // 這個 effect 不會重跑，connectedAtRef 因此不會被後續的 mode 變更
  // 動到，不需要另外分辨「是第一次進 live 還是後續的 mode 變更」。
  const connectedAtRef = useRef<number | null>(null);
  const [elapsedMs, setElapsedMs] = useState(0);
  useEffect(() => {
    if (phase.kind === "live" && connectedAtRef.current === null) {
      connectedAtRef.current = Date.now();
    }
  }, [phase.kind]);
  useEffect(() => {
    if (phase.kind !== "live") return;
    const interval = setInterval(() => {
      if (connectedAtRef.current !== null) {
        // 系統時鐘可能因為 NTP 校時、睡眠喚醒等原因往回跳——夾在 0 避免
        // 負數穿透到 formatElapsed，顯示出詭異的負數秒數。
        setElapsedMs(Math.max(0, Date.now() - connectedAtRef.current));
      }
    }, 1000);
    return () => clearInterval(interval);
  }, [phase.kind]);

  const [bookmarksOpen, setBookmarksOpen] = useState(false);

  const [aiPanelOpen, setAiPanelOpen] = useState(false);
  const remoteAiPanelRef = useRef<RemoteAiPanelHandle>(null);
  // 只負責設 true（unmount + 三個連線事件）；重設回 false 是 RemoteAiPanel
  // 的 submitAgent 的責任（見該檔案的說明）——這裡別自己加重設，會跟那邊
  // 打架。
  const abortRef = useRef(false);
  const [maxAgentSteps, setMaxAgentSteps] = useState(5);

  useEffect(() => {
    getConfig()
      .then((cfg) => {
        setMaxAgentSteps(cfg.max_agent_steps === 0 ? 9999 : (cfg.max_agent_steps ?? 5));
      })
      .catch(() => {});
  }, []);

  // Provider 狀態跟 TerminalView.tsx 同一套模式：掛載時抓一次目前預設的
  // provider，切換靠 ProviderPalette 的 onSwitch 回呼更新，不是靠重新拉取。
  const [activeProvider, setActiveProvider] = useState("");
  const [activeProviderId, setActiveProviderId] = useState("");
  const quotaWindow = useProviderQuota(activeProviderId);
  const [paletteOpen, setPaletteOpen] = useState(false);
  useEffect(() => {
    listProviders()
      .then((list) => {
        const active = list.find((p) => p.is_default) ?? list[0];
        setActiveProvider(active?.display_name ?? "");
        setActiveProviderId(active?.id ?? "");
      })
      .catch(() => {});
  }, []);

  // 一次註冊的 share-viewer://* 事件 handler（下面那個只依賴 [connId] 的
  // effect）需要讀到最新的 submitCommand——跟這個檔案既有的 blocksRef /
  // appendOutputRef 同一套 ref 橋接（也跟 TerminalView.tsx 的
  // submitCommandRef 完全同一個寫法），不為了這個值重新訂閱一次所有事件。
  const submitCommandRef = useRef(submitCommand);
  useEffect(() => { submitCommandRef.current = submitCommand; }, [submitCommand]);
  // 同一個理由：那個 effect 裡的三個連線事件 handler 現在要組譯句字串給
  // RemoteAiPanel.abort(reason) 顯示，若直接在 handler 裡閉包捕捉 `t`，
  // 拿到的會是這個 effect 第一次跑（掛載當下）那份、之後切換語系也不會
  // 跟著換——這個檔案原本就有這個坑（Task 7 之前的 tRef 就是為了它），
  // 這裡重新加回來。
  const tRef = useRef(t);
  useEffect(() => { tRef.current = t; }, [t]);

  // 分頁關閉時把正在跑的 Agent 迴圈中止——這顆 abortRef 同時也是傳給
  // RemoteAiPanel 的 sharedAbortRef（見下方 render），跟它自己內部的
  // unmount-abort effect 各自獨立、互不衝突，兩邊都設是有意的雙重保險。
  useEffect(() => () => { abortRef.current = true; }, []);

  // 卡片列表跟即時窗格共用同一個外層捲動容器，新卡片完成渲染時捲到底部
  // ——跟 TerminalView.tsx 的 blockListRef 同一個手法、同一個理由。
  const scrollAreaRef = useRef<HTMLDivElement | null>(null);
  // 橋接給滾輪鎖定用——那個監聽器註冊在只依賴 [connId] 的 effect 裡，
  // 不能靠閉包讀 isAlternateBuffer。
  const isAlternateBufferRef = useRef(isAlternateBuffer);
  isAlternateBufferRef.current = isAlternateBuffer;
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

  // 提示字元所在的列，讓即時窗格從那一列開始顯示——跟 TerminalView.tsx 的
  // liveTopRows 同一套機制、同一個理由：Windows 主控端不再清空 xterm 緩衝區
  // （見 useTerminalBlocks 的 OSC 133 D 分支），整個 ConPTY 畫面都還在，
  // 從第 0 列開始畫就會把已經變成卡片的舊輸出再顯示一次（實機回報）。
  // 觀看端跟本機分頁餵的是同一串位元組、共用同一個 useTerminalBlocks，
  // 所以必須補上同一套對齊，否則兩邊體驗不一致。
  //
  // 用 viewportY 不是 baseY：xterm 實際是從 viewportY 開始渲染，兩者只在
  // 捲到最底時相同。
  const [liveTopRows, setLiveTopRows] = useState(0);
  const syncLiveTop = useCallback(() => {
    if (hostPlatform !== "windows") return;
    const term = termRef.current;
    const promptAbsRow = promptAbsRowRef.current;
    if (!term || promptAbsRow === null) return;
    const viewportRow = promptAbsRow - term.buffer.active.viewportY;
    setLiveTopRows(Math.max(0, Math.min(term.rows - 1, viewportRow)));
  }, [hostPlatform]);
  syncLiveTopRef.current = syncLiveTop;

  // 見上面 untrackedCommandBoundaryRef 宣告處——這裡才真的賦值，因為
  // setLiveRows 要到這裡才存在。跟 TerminalView.tsx 的同名實作完全一樣，
  // 而且對觀看端來說這條路徑比本機更重要：主控端自己在跑的東西（例如連線
  // 之前就開著的 Claude Code CLI）永遠不會經過這一端的 submitCommand，
  // 「有沒有一個 running 中的區塊」這個撐高訊號因此永遠是 false，窗格會
  // 卡在 MIN_LIVE_ROWS 只有三列高（實機回報）。
  //
  // "start" 先撐到 MAX_LIVE_ROWS 避免輸出被裁掉；"end" 量一次游標實際停在
  // 第幾行，收回剛好放得下的高度——遠端指令的輸出不會變成卡片、也不會被
  // 清空，維持在 MAX 只會在下面留一大截用不到的空白。
  untrackedCommandBoundaryRef.current = (kind) => {
    if (kind === "start") {
      setLiveRows(MAX_LIVE_ROWS);
      return;
    }
    const term = termRef.current;
    const usedRows = term ? term.buffer.active.cursorY + 1 : MIN_LIVE_ROWS;
    setLiveRows(Math.min(MAX_LIVE_ROWS, Math.max(MIN_LIVE_ROWS, usedRows)));
  };

  // xterm.js 沒有公開 API 可以讀字元格高度——這裡讀的是跟 TerminalView.tsx
  // 同一個內部欄位，同一個 escape hatch，這個 repo 已經有先例。
  const cellHeightPx =
    (termState as unknown as { _core?: { _renderService?: { dimensions?: { css?: { cell?: { height?: number } } } } } } | null)
      ?._core?._renderService?.dimensions?.css?.cell?.height || 14 * 1.1;
  const liveHeightPx = Math.round(liveRows * cellHeightPx);
  // 全螢幕程式使用中一律不位移：那類程式自己畫滿整個畫面，沒有東西要裁。
  // 這個 host（.aiterm-remote-terminal__scroll）沒有 padding，所以不需要
  // TerminalView 那邊的 4px 補償。
  const liveTopOffsetPx =
    isAlternateBuffer || liveTopRows <= 0 ? 0 : Math.ceil(liveTopRows * cellHeightPx);
  // 全螢幕程式使用中，即時窗格高度改成剛好塞下主控端目前的實際列數，不再
  // 無條件撐滿容器——容器可能比內容需要的空間大（例如主控端終端機只有
  // 24 列，但觀看端視窗開得比較高），撐滿的話畫面下方會留一大片沒用到
  // 的空白。跟橫向（欄寬）不夠時讓外層捲動、而不是硬擠進去，是同一個
  // 「照實際內容大小顯示，容器決定要不要捲動」的原則；容器真的比內容矮
  // 時（列數比視窗能顯示的還多）一樣交給外層既有的 overflow-y:auto 捲動，
  // 不需要另外處理。
  const altBufferHeightPx = Math.round(hostRows * cellHeightPx);

  // hostPlatform 對一條連線來說實質上不變：onShareViewerGranted 只在第一次
  // 拿到非空 hostOs 時寫入，之後的 resize 通知帶空字串、不會再改它。所以把
  // 當下的值一次性關進這個閉包（傳給 RemoteAiPanel 當 buildRemoteCtx prop）
  // 是安全的。
  // hostPlatform 目前只分 windows / 其它，所以 macOS 主控端會被標成 "linux"，
  // AI 可能因此給出 GNU 而非 BSD 語法的指令（sed -i、readlink -f 等）。granted
  // 事件之後帶更細的 OS 再放寬（spec YAGNI）。
  const buildRemoteCtx = useCallback((): RemoteCtx => ({
    os: hostPlatform === "windows" ? "windows" : "linux",
    shell: null,
    cwd: null,
    recentOutput: readRecentOutput(termRef.current),
  }), [hostPlatform]);

  // WarpInput 送出的整行文字先過一次 AI 前綴檢查——跟本機分頁用同一套
  // parseAiPrefix.ts 規則，不重新猜字首。/agent 與 /ai 都轉交給
  // RemoteAiPanel 自己的 agent 迴圈（透過 ref 呼叫，不在這裡重複狀態）；
  // 其餘走 submitCommand（會建立分段卡片並透過 write 送出）。面板自己管
  // 「是否正在跑」，這裡不用再判斷「任務進行中再送一次＝停止」。
  const handleWarpSubmit = useCallback(
    (cmd: string) => {
      const agentGoal = parseAgentPrefix(cmd);
      const aiGoal = parseAiPrefix(cmd);
      if (agentGoal !== null) { setAiPanelOpen(true); remoteAiPanelRef.current?.submitAgent(agentGoal); return; }
      if (aiGoal !== null) { setAiPanelOpen(true); remoteAiPanelRef.current?.send(aiGoal); return; }
      submitCommand(cmd);
    },
    [submitCommand],
  );

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

    // 跟本機分頁（TerminalView.tsx）同一個理由：`{ stream: true }` 讓跨兩個
    // PTY chunk 被截斷的多位元組 UTF-8 字元（例如中文檔名）正確併回同一個
    // 字元，而不是各自解出替代字元亂碼。這顆 decoder 要跟這個 effect 活得
    // 一樣久（同一個 connId 收到的每個 chunk 都要用同一個實例才有累積
    // 效果），所以宣告在這裡，不是每個 chunk 各建一個。
    const decoder = new TextDecoder("utf-8");

    track(
      onShareViewerGranted(connId, ({ mode, cols, rows, hostOs }) => {
        // 尺寸由主控端說了算——照它給的建立，不用自己的視窗大小。
        // `mode` 為空字串代表這是後續的 resize 通知，不是初次核准。
        if (mode) setPhase({ kind: "live", mode });
        if (hostOs) setHostPlatform(hostOs === "windows" ? "windows" : "other");
        const term = termRef.current;
        if (term && cols > 0 && rows > 0) {
          term.resize?.(cols, rows);
          setHostRows(rows);
        }
      }),
    );

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

    track(
      onShareViewerResync(connId, () => {
        // 清空再接全量重播。漏掉的位元組可能截斷 ANSI 逃脫序列，帶著壞掉
        // 的畫面繼續是不會自己好的。分段卡片內容也是從同一批位元組解析
        // 出來的，漏掉的部分同樣可能讓卡片內容跟畫面對不上——跟本機分頁
        // 執行 clear/cls 時「畫面跟卡片一起清空」是同一個邏輯。
        termRef.current?.clear();
        clearAllBlocksRef.current();
        // 重新同步代表畫面與分段歷史都不再可信，正在跑的 Agent 迴圈接續
        // 判斷會失準——直接中止，並在對話裡說明原因（不是使用者自己按
        // 停止，靜默停止會讓人以為是 bug）。
        abortRef.current = true;
        remoteAiPanelRef.current?.abort(tRef.current.remote_agent_stopped_resync);
      }),
    );

    track(
      onShareViewerControlChanged(connId, (mode) => {
        setPhase({ kind: "live", mode });
        // 失去控制權後 Agent 迴圈沒辦法再送指令，接續會卡住——中止並說明原因。
        if (mode !== "control") {
          abortRef.current = true;
          remoteAiPanelRef.current?.abort(tRef.current.remote_agent_stopped_control_lost);
        }
      }),
    );

    track(
      onShareViewerEnded(connId, (reason) => {
        setPhase({ kind: "ended", reason });
        // 連線都結束了，正在跑的 Agent 迴圈沒有 PTY 可用——中止並說明原因。
        abortRef.current = true;
        remoteAiPanelRef.current?.abort(tRef.current.remote_agent_stopped_ended);
      }),
    );

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
    };
    window.addEventListener("aiterm:font-changed", onFontChanged);

    const onThemeChanged = (e: Event) => {
      const { theme } = (e as CustomEvent).detail as { theme: AppTheme };
      term.options.theme = theme.xterm;
    };
    window.addEventListener("aiterm:theme-changed", onThemeChanged);

    // 即時窗格的滾輪鎖定——跟本機分頁 TerminalView.tsx 完全同一套機制、
    // 同一個理由：這個窗格顯示的是「現在這一段」，要看歷史請用上方的卡片。
    // 緩衝區不再被清空之後，滾輪會捲進 scrollback、把已經有卡片的舊輸出
    // 再露出來一次。捕獲階段 + stopPropagation 是關鍵：xterm 自己在內層
    // .xterm-viewport 上處理滾輪並主動捲動，冒泡階段攔截來不及，
    // preventDefault 也取消不了程式化捲動。
    // 全螢幕程式放行：那類程式自己要用滾輪，而且它們佔滿整個框，沒有卡片
    // 列表可以代為捲動。
    const liveWheelHost = hostRef.current;
    const onLiveWheel = (e: WheelEvent) => {
      if (isAlternateBufferRef.current) return;
      e.preventDefault();
      e.stopPropagation();
      const scroller = scrollAreaRef.current;
      if (scroller) scroller.scrollTop += e.deltaY;
    };
    liveWheelHost.addEventListener("wheel", onLiveWheel, { passive: false, capture: true });

    return () => {
      liveWheelHost.removeEventListener("wheel", onLiveWheel, { capture: true });
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
      <div className="aiterm-status" data-tauri-drag-region>
        {/* whiteSpace: "normal" 只是把「這裡本來就允許換行、不要有人以後
            幫 .aiterm-status-left 加 nowrap」這個意圖寫明白——瀏覽器對
            這個 class 的預設值本來就是 normal，這行不是修正一個既有的
            截斷問題。改成 flex-column：品牌名稱固定在上面一行，連線
            資訊（可能較長、含位址）另起一行，不會互相擠壓換行位置。 */}
        <span
          className="aiterm-status-left"
          data-tauri-drag-region
          style={{ whiteSpace: "normal", display: "flex", flexDirection: "column" }}
        >
          {/* 沿用 AiPanel/index.tsx 既有的漸層文字技巧（同一個
              var(--accent-gradient) CSS 變數、同一套 WebkitBackgroundClip
              /WebkitTextFillColor 組合），不重新設計一套新樣式。 */}
          <span style={{ background: "var(--accent-gradient)", WebkitBackgroundClip: "text", WebkitTextFillColor: "transparent", fontWeight: 700, fontSize: "16px" }}>
            ✨ AITerm
          </span>
          <span>
            {t.remote_terminal_tab} {hostLabel}
          </span>
          <span>{connectionStatusText(t, phase, elapsedMs)}</span>
        </span>
        <span style={{ display: "flex", alignItems: "center", gap: "8px" }}>
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            title={t.remote_terminal_toolbar_connect_button}
            onClick={(e) => {
              e.stopPropagation();
              onConnectClick();
            }}
            style={{ display: "flex", alignItems: "center", gap: "6px" }}
          >
            <LinkIcon size={14} />
            <span>{t.remote_terminal_toolbar_connect_button}</span>
          </button>
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            title={t.term_bookmark_tooltip}
            onClick={(e) => {
              e.stopPropagation();
              setBookmarksOpen(true);
            }}
            style={{ display: "flex", alignItems: "center", gap: "6px" }}
          >
            <span>{t.bookmarks_title}</span>
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            title={phase.kind === "live" && phase.mode === "control" ? t.term_ai_helper_tooltip : t.remote_agent_needs_control}
            disabled={!(phase.kind === "live" && phase.mode === "control")}
            onClick={(e) => { e.stopPropagation(); setAiPanelOpen(true); }}
            style={{ display: "flex", alignItems: "center", gap: "6px" }}
          >
            <SparklesIcon size={14} />
            <span>Ask AI</span>
            {quotaWindow && <QuotaBadge window={quotaWindow} />}
          </button>
        </span>
      </div>
      {bookmarksOpen && (
        <CommandBookmarksPicker
          onSelect={(cmd) => {
            setBookmarksOpen(false);
            window.dispatchEvent(new CustomEvent("warp-fill-command", { detail: { cmd } }));
          }}
          onClose={() => setBookmarksOpen(false)}
        />
      )}
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

      {/* 卡片列表跟即時窗格共用這個外層捲動容器（跟 TerminalView.tsx 的
          blockListRef 同一個結構）——不再是各自獨立、各自有高度上限的
          兩塊，卡片可以無限往下累積，捲動邊界只有這一層。 */}
      <div className="aiterm-remote-terminal__scroll-area" ref={scrollAreaRef}>
        {/* 卡片列表在全螢幕程式（vim/htop/tmux 等）使用中隱藏——跟
            TerminalView.tsx 同一個理由：那類程式必須完整佔滿即時窗格，
            不該被已完成指令的舊卡片跟它搶空間。 */}
        {!isAlternateBuffer && (
          <div className="aiterm-remote-terminal__blocks">
            {/* 分段卡片：跟本機分頁同一套過濾條件（只顯示已結束且已完成
                ANSI 解析的），複用 TerminalBlockCard——不傳 onAskAi，
                Ask AI 按鈕本身是 `{isFailed && onAskAi && (...)}` 條件
                渲染，不傳就不會出現；block.gitInfo 永遠是 undefined
                （這裡從不呼叫 setBlockGitInfo），git 徽章同理自然不
                出現。 */}
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
        )}

        {/* 外層框住並裁切即時畫面；hostRef 本身內部永遠固定高度，
            這樣它（以及 xterm 自己內部的尺寸監聽）永遠不會因為這一層
            高度變化而看到容器尺寸改變——只有這一層的高度會變。跟
            TerminalView.tsx 的 .aiterm-live-frame 完全同一套機制，包含
            全螢幕程式（vim/htop 等）使用中撐滿、不裁切這件事：拿掉自動
            縮放字體後，若仍然把高度夾在 MAX_LIVE_ROWS 並用 overflow:clip
            硬裁，全螢幕程式會被裁到只剩最後 16 行、其餘完全看不到也滑
            不到——這是實機審查抓到的迴歸，不是刻意的設計。 */}
        <div
          className="aiterm-remote-terminal__live-frame"
          style={{
            // 全螢幕程式使用中改用 altBufferHeightPx（主控端目前實際列數
            // 換算出的像素高度），不是無條件撐滿容器的 100%——容器可能比
            // 內容需要的空間大（例如主控端只有 24 列，但觀看端視窗開得
            // 比較高），撐滿的話畫面下方會留一大片沒用到的空白（實機
            // 回報過的問題）。跟本機終端機（TerminalView.tsx）不同：那邊
            // 用 FitAddon 讓「終端機列數」永遠等於「容器裝得下的列數」，
            // 兩者天生一致，撐滿容器不會有多餘空白；這裡的列數是主控端
            // 說了算，觀看端的容器大小跟它無關，撐滿容器反而可能比內容
            // 需要的還大。
            height: isAlternateBuffer ? `${altBufferHeightPx}px` : `${liveHeightPx}px`,
            width: "calc(100% - 16px)",
            margin: "6px 8px",
            boxSizing: "border-box",
            flexShrink: 0,
            // 全螢幕程式使用中改成 visible：`clip` 會把內容硬裁在固定
            // 220px 高的 hostRef 裡，不裁的話反而要讓 hostRef 自己撐滿
            // 100%（見下面的 hostRef style），這裡的 overflow 只是配合
            // 這個切換，不是又要開放捲動。
            overflow: isAlternateBuffer ? "visible" : "clip",
            // 下面的 host 靠絕對定位往上位移對齊提示字元，這一層要當定位基準。
            position: liveTopOffsetPx > 0 ? "relative" : undefined,
          }}
        >
          <div
            className="aiterm-remote-terminal__scroll"
            ref={hostRef}
            style={{
              height: isAlternateBuffer ? "100%" : "220px",
              width: "100%",
              boxSizing: "border-box",
              // 見 liveTopRows：把提示字元那一列推到窗格頂端。
              ...(liveTopOffsetPx > 0
                ? { position: "absolute" as const, top: -liveTopOffsetPx, left: 0, right: 0, width: "auto" }
                : null),
            }}
          />
        </div>
      </div>

      {/* 全螢幕程式使用中隱藏——跟本機終端機同一個理由：這類程式的輸入
          直接打進上面的即時畫面，不透過這個獨立的指令輸入框，留著只會
          白白佔用本該讓給即時窗格的空間。 */}
      {!isAlternateBuffer && (
        <WarpInput
          onSubmit={handleWarpSubmit}
          disabled={!(phase.kind === "live" && phase.mode === "control")}
        />
      )}

      <RemoteAiPanel
        ref={remoteAiPanelRef}
        isOpen={aiPanelOpen}
        onClose={() => setAiPanelOpen(false)}
        connId={connId}
        buildRemoteCtx={buildRemoteCtx}
        submitCommand={(cmd, cb) => submitCommandRef.current(cmd, cb)}
        isControl={phase.kind === "live" && phase.mode === "control"}
        maxSteps={maxAgentSteps}
        providerName={activeProvider}
        providerId={activeProviderId}
        onOpenProviderPalette={() => setPaletteOpen(true)}
        sharedAbortRef={abortRef}
      />
      {paletteOpen && (
        <ProviderPalette
          onClose={() => setPaletteOpen(false)}
          onSwitch={(p) => { setActiveProvider(p.display_name); setActiveProviderId(p.id); }}
        />
      )}
    </div>
  );
}

/**
 * 讀 xterm buffer 末段當 AI 情境；term 未建立回 null。從游標所在列往回
 * 掃到滿 `maxChars` 或掃到頂為止，維持原本的上到下順序回傳。
 *
 * export 出去（跟 `formatElapsed` 一樣）方便直接單元測試，Fast Refresh 的
 * lint 規則會抗議多一個非元件的具名匯出——跟 `formatElapsed` 同一個取捨，
 * 沿用同一條 disable。
 */
// eslint-disable-next-line react-refresh/only-export-components -- 見上方註解：純函式匯出換取可直接單元測試，不影響正式建置。
export function readRecentOutput(term: Terminal | null, maxChars = 4000): string | null {
  if (!term) return null;
  const buf = term.buffer.active;
  const bottom = buf.baseY + buf.cursorY;
  const lines: string[] = [];
  let chars = 0;
  for (let i = bottom; i >= 0 && chars < maxChars; i--) {
    const s = buf.getLine(i)?.translateToString(true) ?? "";
    lines.push(s);
    chars += s.length + 1;
  }
  lines.reverse();
  const joined = lines.join("\n").trimEnd();
  return joined.length > 0 ? joined : null;
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
 * 工具列左側的連線狀態片語，依 `phase` 三態切換。`elapsedMs` 只在
 * `phase.kind === "live"` 時才會被用到，其餘兩態忽略它。
 */
function connectionStatusText(t: Translations, phase: Phase, elapsedMs: number): string {
  if (phase.kind === "waiting") return t.remote_terminal_waiting_approval;
  if (phase.kind === "ended") return t.remote_terminal_toolbar_ended;
  const modeLabel = phase.mode === "read_only" ? t.remote_terminal_read_only : t.remote_terminal_toolbar_control_mode;
  return `${t.remote_terminal_toolbar_connected_prefix} ${formatElapsed(elapsedMs)} · ${modeLabel}`;
}

/**
 * 把毫秒數轉成「12s」/「3m45s」/「1h05m」這種簡短格式。跟
 * `TerminalBlockCard.tsx` 裡既有的 `formatDuration` 邏輯類似但獨立寫一份
 * （不跨檔案匯出私有函式）：那邊是給單一指令的執行時間用，通常不會超過
 * 一小時，這裡是連線總時間，可能開很久，需要多處理小時這一級，用途不同
 * 分開寫更清楚。
 *
 * 秒數欄位一律補零到兩位（"3m05s" 不是 "3m5s"）——不補零的話，個位數的
 * 秒數在 9s → 10s 那一格會讓字串長度突然變化，畫面上看起來像閃一下；
 * 小時分支的 `remMinutes` 本來就有補零，這裡補齊讓兩個分支一致。
 *
 * export 出去給 `index.test.tsx` 直接單元測試：分鐘/小時這兩個分支只
 * 靠元件間接測會被 <60s 的案例蓋過去，測不到。這會讓這個檔案同時匯出
 * 元件跟一個純函式，Fast Refresh 的 lint 規則會抗議——這裡刻意接受這個
 * 副作用（開發時這個檔案偶爾會整個重新載入，而不是熱替換），換取這個
 * 純函式能被直接單元測試，不用另開一個檔案。
 */
// eslint-disable-next-line react-refresh/only-export-components -- 見上方註解：純函式匯出換取可直接單元測試，不影響正式建置。
export function formatElapsed(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000);
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes < 60) return `${minutes}m${String(seconds).padStart(2, "0")}s`;
  const hours = Math.floor(minutes / 60);
  const remMinutes = minutes % 60;
  return `${hours}h${String(remMinutes).padStart(2, "0")}m`;
}
