import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useLocale } from "../contexts/LocaleContext";
import { findAppCaret } from "../lib/terminalCaret";
import { collapseWholeStringRepeat } from "../lib/collapseWholeStringRepeat";
import { ResizeRepaintGate } from "../lib/resizeRepaintGate";
import { listen } from "@tauri-apps/api/event";
import { homeDir } from "@tauri-apps/api/path";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import "@xterm/xterm/css/xterm.css";

import {
  closePty,
  createPty,
  getPtyRecentOutput,
  onPtyData,
  resizePty,
  writePastedFile,
  writePty,
} from "../ipc/pty";
import {
  invokeAiQuery,
  type AiStreamEvent,
} from "../ipc/ai";
import { getConfig, type ExecutionMode, type SubmitShortcut } from "../ipc/config";
import { getSessionCwd } from "../ipc/fs";
import { enterpriseCompleteTask, enterpriseOnComplete } from "../ipc/enterprise";
import { useTerminalBlocks } from "../hooks/useTerminalBlocks";
import { useAgentMission } from "../hooks/useAgentMission";
import { useTelegramRemoteControl } from "../hooks/useTelegramRemoteControl";
import { listProviders } from "../ipc/provider";
import { parseAiPrefix, parseAgentPrefix } from "./parseAiPrefix";
import { CommandPreview } from "./CommandPreview";
import { StreamingIndicator } from "./StreamingIndicator";
import { AgentStatusBar, type AgentPhase } from "./AgentStatusBar";
import { AiPanel } from "./AiPanel";
import { ProviderPalette } from "./ProviderPalette";
import { QuotaBadge } from "./QuotaBadge";
import { SharePanel } from "./SharePanel";
import { useProviderQuota } from "../hooks/useProviderQuota";
import { WarpInput } from "./WarpInput";
import { FileExplorer } from "./FileExplorer/FileExplorer";
import { CommandBookmarksPicker, addBookmark } from "./CommandBookmarks";
import { getActiveTheme, type AppTheme } from "../lib/themes";
import { readLineExcludingInlinePrediction } from "../lib/terminalLinePrediction";
import { attentionForExitCode, type AttentionKind } from "../lib/terminalAttention";
import { RobotIcon, SparklesIcon, SmartphoneIcon } from "./Icons";
import { TerminalBlockCard } from "./TerminalBlockCard";
import { findNextBlockMatch, findPreviousBlockMatch, type BlockSearchCursor } from "../lib/blockSearch";
import { summarizeCommands } from "../lib/summarizeTab";
import { reportAgentStep, type AgentStepInfo } from "../lib/agentStepReport";
import { runAgentLoop, INITIAL_PREVIEW, type PreviewState } from "../lib/agentLoop";
import { getGitBlockInfo } from "../ipc/vcs";
import { isClaudeCommand } from "../lib/claudeCommand";
import { CloseConfirmDialog } from "./CloseConfirmDialog";
import "./TerminalView.css";

/**
 * Resolve a pasted/dropped File to a real filesystem path. Uses `.path` when
 * the webview provides one (OS drag-and-drop); otherwise (clipboard paste,
 * which never carries a real path) writes the bytes to a temp file.
 */
async function resolvePastedFilePath(file: File): Promise<string> {
  const existingPath = (file as File & { path?: string }).path;
  if (existingPath) return existingPath;

  const dataUrl = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
  const base64Data = dataUrl.slice(dataUrl.indexOf(",") + 1);
  return writePastedFile(file.name, base64Data);
}

export interface TerminalViewProps {
  isActive?: boolean;
  onToggleSidebar?: () => void;
  isSidebarOpen?: boolean;
  /** Called once with the backend-assigned PTY session ID when the PTY is created. */
  onSessionCreated?: (sessionId: string) => void;
  /** If set, this tab adopts an already-existing backend PTY session instead
   *  of creating a new one — used for tabs spawned by the MCP tool server's
   *  agent-coordination tools (spawn_tab), where the backend already has a
   *  live session before any TerminalView exists to render it. */
  externalSessionId?: string;
  /** Reports whether this tab's most recent command block is currently
   *  running (as opposed to completed/failed/no blocks yet). Used by
   *  TerminalApp to decide whether to steal focus for a newly agent-spawned
   *  tab — never switch away from a tab the user is actively watching run
   *  something. */
  onRunningChange?: (isRunning: boolean) => void;
  /** If set, the PTY starts in this directory (overrides last-cwd from localStorage). */
  initialCwd?: string;
  /** If set, the agent loop starts automatically after the PTY is ready. */
  initialMission?: { goal: string; maxSteps: number };
  /** Enterprise task metadata — triggers on_complete actions when the mission finishes. */
  enterpriseTask?: { taskId: string; workBranch: string; onComplete: unknown };
  /** 每個 agent 步驟完成時呼叫，回報首頁「進行中的任務」的進度——
   *  不限企業任務，Telegram 遠端指令、終端機內 `/agent`、WarpInput 送出的 mission 都會回報。 */
  onAgentProgress?: (done: number, total: number) => void;
  /** agent mission 結束時呼叫（不論成功或失敗），讓首頁清掉這個分頁的進度——
   *  「進行中的任務」只該列真的在跑的，跑完/失敗的訊號另外由 onAttention 負責。 */
  onMissionEnd?: () => void;
  /** Called with a freshly generated AI summary of this tab's conversation, for the title bar. */
  onSummaryUpdate?: (summary: string) => void;
  /** 工作目錄變了就回報一次。上層用它更新分頁狀態並記進最近專案。 */
  onCwdChange?: (cwd: string) => void;
  /** 這個分頁發生了需要使用者注意的事。TerminalView 一律回報，
   *  「這個分頁是不是 active」與「視窗有沒有 focus」都由 TerminalApp 判斷——
   *  避免那些條件在 xterm / PTY 事件的 closure 裡變 stale。 */
  onAttention?: (kind: AttentionKind) => void;
  /** 使用者在這個分頁執行了 Claude Code。用來提示他設定 terminal bell。 */
  onClaudeDetected?: () => void;
  /**
   * 這個分頁是否注入 Claude Code 橋接環境變數。
   * 環境變數只能在 PTY spawn 的瞬間決定，所以這個值在分頁建立後改變沒有效果。
   */
  claudeBridge?: boolean;
  /** 這個分頁的穩定識別碼（`tab.id`），當作 Telegram Remote 的 ownerKey。 */
  tabId: string;
  /**
   * 目前誰擁有 Remote（`TerminalApp` 的 `remoteTabId`）。null = 沒有人。
   * isRemoteEnabled 由 `tabId === remoteOwner` 推導，跟這個分頁在畫面上
   * 看不看得到無關（切到首頁也一樣算數，這正是本欄位存在的理由：修好
   * Telegram 遠端遙控在首頁按鈕出現後的回歸）。
   */
  remoteOwner?: string | null;
  /**
   * 使用者切換這個分頁的 Remote 開關時呼叫，讓 TerminalApp 更新
   * remoteTabId（天然互斥：在這裡開會自動關掉原本開著的那個分頁）。
   */
  onRemoteOwnerChange?: (owner: string | null) => void;
  registerCloseGuard?: (tabId: string, guard: () => Promise<boolean>) => void;
  unregisterCloseGuard?: (tabId: string) => void;
}

// The live terminal pane's visible height shrinks to just the current content
// (down to MIN_LIVE_ROWS) instead of always reserving MAX_LIVE_ROWS worth of
// mostly-empty space. The underlying xterm host div stays a fixed MAX_LIVE_ROWS
// tall at all times — only an outer wrapper's CSS height is animated, so this
// never touches xterm's actual row count / triggers a PTY resize.
const MIN_LIVE_ROWS = 3;
const MAX_LIVE_ROWS = 16;

/** Must stay in sync with `.aiterm-terminal-root`'s `padding` in
 *  TerminalView.css — the prompt-row offset has to account for it, see
 *  liveTopOffsetPx. */
const TERMINAL_HOST_PADDING_PX = 4;

const SEARCH_OPTS = {
  regex: false,
  caseSensitive: false,
  wholeWord: false,
  decorations: {
    matchBackground: '#ffff0040',
    matchBorder: '#ffff00',
    matchOverviewRuler: '#ffff00',
    activeMatchBackground: '#ff990040',
    activeMatchBorder: '#ff9900',
    activeMatchColorOverviewRuler: '#ff9900',
  },
};

export function TerminalView({ isActive = true, onToggleSidebar, isSidebarOpen = true, onSessionCreated, externalSessionId, onRunningChange, initialCwd, initialMission, enterpriseTask, onAgentProgress, onMissionEnd, onSummaryUpdate, onCwdChange, onAttention, onClaudeDetected, claudeBridge, tabId, remoteOwner = null, onRemoteOwnerChange, registerCloseGuard, unregisterCloseGuard }: TerminalViewProps) {
  type ViewTab = "terminal" | "files";
  const [viewTab, setViewTab] = useState<ViewTab>("terminal");
  const navigate = useNavigate();
  const { t, locale } = useLocale();
  const hostRef = useRef<HTMLDivElement>(null);
  const blockListRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<string>("initializing…");
  const [preview, setPreview] = useState<PreviewState>(INITIAL_PREVIEW);
  const previewRef = useRef<PreviewState>(INITIAL_PREVIEW);
  previewRef.current = preview;

  const [panelOpen, setPanelOpen] = useState(false);
  const [bookmarksOpen, setBookmarksOpen] = useState(false);
  const [sessionId, setSessionId] = useState<string>("");
  const [displayCwd, setDisplayCwd] = useState<string>("");
  const lastCwdRef = useRef<string>("");

  // Bridge onCwdChange into a ref，理由跟 onSummaryUpdateRef 一樣：TerminalApp
  // 每次 render 都會傳新的 inline arrow，若放進下面輪詢 effect 的 dep array，
  // 會害那個 effect 每次 render 就重跑一次（重置 lastSaved/homePath、重建
  // setInterval），這裡刻意不動那段既有邏輯。
  const onCwdChangeRef = useRef(onCwdChange);
  useEffect(() => { onCwdChangeRef.current = onCwdChange; }, [onCwdChange]);

  // Persist the active terminal's CWD to localStorage so it can be
  // restored on the next session. Also updates the status bar CWD display.
  useEffect(() => {
    if (!sessionId) return;
    let lastSaved = "";
    let homePath = "";
    homeDir().then((h) => { homePath = h.replace(/\\/g, "/").replace(/\/$/, ""); }).catch(() => {});
    const id = setInterval(async () => {
      try {
        const cwd = await getSessionCwd(sessionId);
        if (cwd && cwd !== lastSaved) {
          lastSaved = cwd;
          localStorage.setItem("aiterm_last_cwd", cwd);
          const normalized = cwd.replace(/\\/g, "/");
          const pretty = homePath && normalized.startsWith(homePath)
            ? "~" + normalized.slice(homePath.length)
            : normalized;
          lastCwdRef.current = cwd;
          setDisplayCwd(pretty);
          onCwdChangeRef.current?.(cwd);
        }
      } catch { /* session may not be ready yet */ }
    }, 2000);
    return () => clearInterval(id);
  }, [sessionId]);

  const panelOpenRef = useRef(false);
  useEffect(() => {
    panelOpenRef.current = panelOpen;
  }, [panelOpen]);

  // Streaming state
  const [streamText, setStreamText] = useState("");
  const streamingRef = useRef(false);
  const executionModeRef = useRef<ExecutionMode>("always-confirm");
  
  const { agentMission, startMission, stopMission, addTokens } = useAgentMission();

  // Provider status badge
  const [activeProvider, setActiveProvider] = useState<string>("");
  /** 目前 provider 的 id。顯示名稱不能拿來查配額——後端是用 id 找設定的。 */
  const [activeProviderId, setActiveProviderId] = useState<string>("");
  /** 常駐配額徽章的代表窗；null 就不顯示。 */
  const quotaWindow = useProviderQuota(activeProviderId);
  const [paletteOpen, setPaletteOpen] = useState(false);

  // Execution mode and shortcut are read once and cached; re-fetched when we return from settings.
  const [submitShortcut, setSubmitShortcutState] = useState<SubmitShortcut>("enter");
  const maxAgentStepsRef = useRef<number>(5);

  // Refs bridged into the useEffect closure.
  const termRef = useRef<Terminal | null>(null);
  const [termState, setTermState] = useState<Terminal | null>(null);
  const sessionRef = useRef<string | null>(null);
  // Agent 卡住偵測用：PTY 最後一次吐出東西的時間。卡在 heredoc>／等輸入的
  // 互動程式是完全安靜的，而跑得好好的長指令會持續有輸出——用「安靜多久」
  // 區分兩者，比固定逾時準得多，也不會誤殺跑很久但正常的工作。
  const lastPtyOutputAtRef = useRef(Date.now());
  // Snapshot of the rendered line (prompt + anything already typed) taken the
  // moment a fresh input line starts — see the onData handler below for why.
  // null means "no line in progress, capture a fresh snapshot on next input".
  const lineStartSnapshotRef = useRef<string | null>(null);
  // True while a bracketed paste has landed on the prompt line but Enter
  // hasn't been pressed yet. Both zsh and PowerShell/PSReadLine render
  // pasted-but-unsubmitted text with special styling (reverse video / dim),
  // and redrawing that line in response to a PTY resize (SIGWINCH) — even
  // one triggered by a legitimate, unrelated layout change — can leave it
  // corrupted/invisible. See pendingResizeRef below for how this is used.
  const hasUnsubmittedPasteRef = useRef(false);
  const unsubmittedPasteTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  // A resize held back from the PTY until it's safe/possible to send: either
  // hasUnsubmittedPasteRef was true (flushed on Enter or the safety timeout
  // below), or — the other case this ref covers — term.onResize fired
  // before createPty()'s promise had resolved (sessionRef.current still
  // null), so there was no session id to resize yet. xterm.js's own
  // rows/cols are already correct at that point (fit() already ran); only
  // the downstream resizePty() call was impossible to make. Without holding
  // this back and retrying once the session exists, that resize is lost
  // permanently: FitAddon only calls terminal.resize() (which is what fires
  // term.onResize) when the newly measured size actually differs from
  // xterm's CURRENT rows/cols, so once xterm reflects the correct size, any
  // later resize to roughly the same pane size computes the same value and
  // never fires onResize again — no further retry would ever happen,
  // leaving the PTY (and whatever a fullscreen TUI app queries) stuck at
  // its stale creation-time size, immune to the user resizing the window.
  const pendingResizeRef = useRef<{ rows: number; cols: number } | null>(null);

  // Find in Buffer state
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchMatchInfo, setSearchMatchInfo] = useState<string>("");
  const [blockSearchCursor, setBlockSearchCursor] = useState<BlockSearchCursor | null>(null);
  const searchAddonRef = useRef<SearchAddon | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  /** IME（注音等）組字進行中。見寫入路徑的 isWindows 分支。 */
  const isComposingRef = useRef(false);
  /**
   * 應用程式是否用 ESC[?25l 把游標藏起來（DECTCEM）。
   *
   * Claude Code 這類 Ink/TUI 會自己畫一個反白方塊當游標、把真游標關掉，於是
   * xterm 的 buffer 游標就停在「最後一次繪製結束的地方」——實測是狀態列右緣
   * （96 欄的終端量到 cursor col=95），跟使用者眼中的輸入框毫無關係。
   * xterm 的組字 UI（.composition-view 與 helper textarea）都跟著 buffer 游標走，
   * 注音的預覽字與待選字視窗因此跑到畫面另一頭。見 TerminalView.css 的對應規則。
   */
  const [cursorHidden, setCursorHidden] = useState(false);

  const fitAddonRef = useRef<FitAddon | null>(null);

  // Deliberately does NOT call fitAddon.fit() here (unlike the Files-tab ->
  // Terminal-tab repaint fix below, where fit() is genuinely needed to
  // recover from a stale zero-size measurement). Root-caused via DevTools
  // diagnostic logging: fit() re-measures hostRef's parent and computes cols
  // based partly on FitAddon's own scrollbar-width estimate, which shifts
  // once a long command has pushed enough content into scrollback for xterm
  // to consider a scrollbar needed — even though our CSS clips it out of
  // view. That's enough to make fit() see a different column count and call
  // term.resize(), which fires a real PTY resize. On Windows, ConPTY
  // responds to a resize by re-transmitting its currently-visible screen
  // content (its own screen-buffer model, unlike a Unix pty) — arriving
  // ~60ms later and overwriting the freshly-cleared prompt with stale
  // output. term.refresh() alone triggers no resize and no such replay.
  const forceLiveRepaint = useCallback(() => {
    const term = termRef.current;
    if (!term) return;
    term.refresh(0, term.rows - 1);
  }, []);


  // TerminalApp 傳進來的是 inline arrow function，每次 render 都是新身分。
  // 直接把它放進 useTerminalBlocks 的依賴會讓 OSC handler 每次 render
  // 重新註冊。橋接成 ref，對外露出一個永久穩定的 emitAttention——
  // 與本檔 submitCommandRef / beginTrackedBlockRef 同樣的理由與作法。
  const onAttentionRef = useRef(onAttention);
  useEffect(() => { onAttentionRef.current = onAttention; }, [onAttention]);

  const emitAttention = useCallback((kind: AttentionKind) => {
    onAttentionRef.current?.(kind);
  }, []);

  const handleCommandSettled = useCallback((exitCode: number) => {
    emitAttention(attentionForExitCode(exitCode));
  }, [emitAttention]);

  const onClaudeDetectedRef = useRef(onClaudeDetected);
  useEffect(() => { onClaudeDetectedRef.current = onClaudeDetected; }, [onClaudeDetected]);

  const handleCommandStarted = useCallback((cmd: string) => {
    if (isClaudeCommand(cmd)) onClaudeDetectedRef.current?.();
  }, []);

  // 保底機制——正常情況下，遠端觀看者送進來的指令會被 useTerminalBlocks
  // 內部的 recoverUntrackedCommand 從畫面內容還原出指令文字，直接變成
  // 跟本機一樣的完整卡片（見 useTerminalBlocks.ts 的 recoverUntrackedCommand
  // 與 docs/superpowers/specs/2026-08-27-remote-command-text-recovery-design.md），
  // 走的是 beginTrackedBlock 那條正常路徑，不會用到這個 ref。這個 ref
  // 只在還原失敗的少見情況下才會被呼叫（例如這個連線還沒收過任何 OSC 133
  // B 標記），確保即使拿不到指令文字，即時窗格至少不會因為完全沒有信號而
  // 裁切掉遠端指令的輸出。
  // 宣告在這裡是因為 useTerminalBlocks 呼叫點在 setLiveRows 宣告之前，
  // 真正賦值要等 liveRows 宣告完才能做（見下方賦值處），先用 ref 佔位。
  const untrackedCommandBoundaryRef = useRef<((kind: "start" | "end") => void) | null>(null);
  const handleUntrackedCommandBoundary = useCallback((kind: "start" | "end") => {
    untrackedCommandBoundaryRef.current?.(kind);
  }, []);

  // Same placeholder-ref pattern as untrackedCommandBoundaryRef above: the
  // real implementation needs state declared further down, but the OSC 133 B
  // callback that drives it is passed into useTerminalBlocks up here.
  const syncLiveTopRef = useRef<(() => void) | null>(null);

  // Absolute buffer row of the current prompt, from OSC 133 B. Only used on
  // Windows, where the xterm buffer is never cleared and the live pane has to
  // work out for itself which row to start showing from — see liveTopRows.
  const promptAbsRowRef = useRef<number | null>(null);
  const handlePromptStart = useCallback((absoluteRow: number) => {
    promptAbsRowRef.current = absoluteRow;
    syncLiveTopRef.current?.();
  }, []);

  const { blocks, isAlternateBuffer, submitCommand, beginTrackedBlock, appendOutput, setBlockGitInfo, finalizeBlock } = useTerminalBlocks(
    sessionId,
    termState,
    lastCwdRef,
    forceLiveRepaint,
    handleCommandSettled,
    handleCommandStarted,
    undefined,
    undefined,
    handleUntrackedCommandBoundary,
    handlePromptStart,
  );

  useEffect(() => {
    const latest = blocks[blocks.length - 1];
    onRunningChange?.(latest?.status === "running");
  }, [blocks, onRunningChange]);

  // 關閉確認：ref 是必要的，不是風格選擇。guard 只註冊一次，若閉包捕捉
  // 當下的值，之後開始跑的指令它都看不到，會在沒有任何錯誤訊號的情況下
  // 直接放行——功能等於靜默失效。
  const isBusyRef = useRef(false);
  isBusyRef.current = blocks[blocks.length - 1]?.status === "running";
  const missionActiveRef = useRef(false);
  missionActiveRef.current = agentMission?.active ?? false;

  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const closeResolveRef = useRef<((canClose: boolean) => void) | null>(null);

  const handleCloseConfirm = useCallback((canClose: boolean) => {
    setShowCloseConfirm(false);
    closeResolveRef.current?.(canClose);
    closeResolveRef.current = null;
  }, []);

  useEffect(() => {
    if (!tabId || !registerCloseGuard) return;
    registerCloseGuard(tabId, () => {
      // 閒置的終端機沒有進行中的工作可失去，不要打擾使用者。
      if (!isBusyRef.current && !missionActiveRef.current) {
        return Promise.resolve(true);
      }
      return new Promise<boolean>((resolve) => {
        closeResolveRef.current = resolve;
        setShowCloseConfirm(true);
      });
    });
    return () => { unregisterCloseGuard?.(tabId); };
  }, [tabId, registerCloseGuard, unregisterCloseGuard]);

  /**
   * 應用程式自己在畫面上畫的游標（一格反白）。
   *
   * 全螢幕 TUI 會用 ESC[?25l 關掉終端機的真游標，改用一格反白當視覺 caret。
   * 真游標一關，就沒人負責把它擺回 caret：Windows 的 ConPTY 讓它停在最後寫到
   * 的那格（實測 col 51、col 81 都出現過，不是固定在最後一欄）。xterm 的 IME
   * 組字 UI 跟著真游標走，注音因此跑到畫面別處。
   *
   * 找得到這格反白，就等於拿到了可信的 caret 位置——這是個正面訊號，比先前用
   * 「游標是不是在最後一欄」去反推可靠得多。
   */
  const [appCaret, setAppCaret] = useState<{ x: number; y: number } | null>(null);

  /**
   * 要不要把 xterm 的組字 UI 釘到 appCaret 上，不跟著真游標。
   *
   * - alternate buffer：一般 shell 的游標永遠可信，不該碰。
   * - 游標被藏起來：vim 這類會顯示游標的 TUI 自己就把游標放對位置了。
   * - 找得到自繪 caret：沒找到就沒有更好的位置可用，維持原狀不動。
   *
   * 位置用 CSS 變數傳給 TerminalView.css，而不是直接寫 inline style：xterm 每次
   * 繪製都會覆寫組字元素的 style.left/top，只有 `!important` 蓋得過，而
   * `!important` 必須寫在樣式表裡。
   */
  const parkIme = isAlternateBuffer && cursorHidden && appCaret !== null;

  // 只在「TUI 且游標被藏起來」時才掃描——一般情況真游標可信，掃了也是白費。
  const scanCaret = isAlternateBuffer && cursorHidden;
  useEffect(() => {
    const host = hostRef.current;
    if (!host || !scanCaret) {
      setAppCaret(null);
      return;
    }
    const apply = () => {
      const term = termRef.current;
      const cell = (term as unknown as {
        _core?: { _renderService?: { dimensions?: { css?: { cell?: { width?: number; height?: number } } } } };
      } | null)?._core?._renderService?.dimensions?.css?.cell;
      if (!term || !cell?.width || !cell.height) return;
      const caret = findAppCaret(term);
      setAppCaret((prev) =>
        prev?.x === caret?.x && prev?.y === caret?.y ? prev : caret && { x: caret.x, y: caret.y },
      );
      if (!caret) return;
      // 位置一律用格子尺寸算，不量 DOM：host 的 ResizeObserver 先於 xterm 自己的
      // resize 觸發（fit 還被 debounce 120ms），量到的是改變前的舊值，而且之後不會
      // 再有第二次通知修正它——實測就這樣把 textarea 釘在畫面中間。
      host.style.setProperty("--ime-park-left", `${Math.max(0, caret.x * cell.width)}px`);
      host.style.setProperty("--ime-park-top", `${Math.max(0, caret.y * cell.height)}px`);
    };
    apply();
    // 剛切進 alternate buffer 時格子尺寸可能還沒量好，補一次。
    const retry = setTimeout(apply, 300);
    // caret 會隨打字移動，所以每次重繪都要重算。TUI 每秒重繪好幾次，全螢幕掃一遍
    // 雖然便宜也沒必要每幀做，節流到 ~10Hz。
    //
    // 必須補跑最後一次（trailing edge）：Claude Code 送出文字後會連續重繪好幾幀，
    // 最後那幾幀落在節流窗內就被丟掉，而之後畫面靜止、不再有重繪，掃描就永遠不會
    // 再跑——appCaret 會一直停在送出前的舊值。實測就是這樣讓 caret 卡在第 2 欄，
    // 下一段組字從行首疊上去把已送出的字蓋掉。
    let lastScan = 0;
    let trailing: ReturnType<typeof setTimeout> | null = null;
    const scanThrottled = () => {
      const wait = 100 - (performance.now() - lastScan);
      if (wait <= 0) {
        lastScan = performance.now();
        apply();
        return;
      }
      if (trailing) return;
      trailing = setTimeout(() => {
        trailing = null;
        lastScan = performance.now();
        apply();
      }, wait);
    };
    const onRender = termRef.current?.onRender(scanThrottled);
    const onResize = termRef.current?.onResize(apply);
    return () => {
      clearTimeout(retry);
      if (trailing) clearTimeout(trailing);
      onRender?.dispose();
      onResize?.dispose();
      host.style.removeProperty("--ime-park-top");
      host.style.removeProperty("--ime-park-left");
    };
  }, [scanCaret]);

  // 終端機的 bell（\x07）。CLI 工具停下來等使用者回答時多半會敲一次——
  // 這是全螢幕 TUI（Claude Code、vim、lazygit）執行期間唯一可用的訊號，
  // 因為 shell 在那段期間把整個 TUI 視為「一個還在跑的指令」，
  // OSC 133 D 要等它退出才會發出。
  useEffect(() => {
    if (!termState) return;
    const disposable = termState.onBell(() => emitAttention("waiting"));
    return () => disposable.dispose();
  }, [termState, emitAttention]);

  // Bridge submitCommand into a ref so the stale term.onData closure can access the latest version
  const submitCommandRef = useRef(submitCommand);
  useEffect(() => { submitCommandRef.current = submitCommand; }, [submitCommand]);

  // Bridge beginTrackedBlock into a ref for the same reason — onData is registered
  // inside a mount-once effect and would otherwise close over a stale version.
  const beginTrackedBlockRef = useRef(beginTrackedBlock);
  useEffect(() => { beginTrackedBlockRef.current = beginTrackedBlock; }, [beginTrackedBlock]);

  // Abort signal for agent loop — set to true to stop the loop
  const agentAbortRef = useRef(false);
  const agentMissionRef = useRef(agentMission);
  useEffect(() => { agentMissionRef.current = agentMission; }, [agentMission]);

  // Bridge blocks into a ref so the stale closure can check if a command is running
  const blocksRef = useRef(blocks);
  useEffect(() => { blocksRef.current = blocks; }, [blocks]);

  // Same stale-closure bridge for the resize-repaint gate below — it lives
  // inside the PTY-session effect, which doesn't re-run when alternate-buffer
  // state flips.
  const isAlternateBufferRef = useRef(isAlternateBuffer);
  useEffect(() => { isAlternateBufferRef.current = isAlternateBuffer; }, [isAlternateBuffer]);
  const resizeRepaintGateRef = useRef<ResizeRepaintGate | null>(null);
  if (!resizeRepaintGateRef.current) resizeRepaintGateRef.current = new ResizeRepaintGate();

  // Generate a one-time identifying tab title from the first executed
  // command(s). Debounced so rapid successive commands are captured together;
  // the ref guard makes it fire at most once per terminal session (the view
  // stays mounted across tab switches). Summarizes command text only — see
  // summarizeCommands. Best-effort: the guard latches once regardless of
  // outcome, so a failed first attempt leaves the tab showing its plain name
  // until the app restarts.
  const summaryGeneratedRef = useRef(false);
  // Bridge onSummaryUpdate into a ref: TerminalApp passes a new inline arrow on
  // every render, so keeping it in the trigger effect's dep array would re-run
  // (and reset the 1.5s debounce) on every parent render — starving the summary
  // during busy/agent scenarios that re-render sub-1.5s.
  const onSummaryUpdateRef = useRef(onSummaryUpdate);
  useEffect(() => { onSummaryUpdateRef.current = onSummaryUpdate; }, [onSummaryUpdate]);
  useEffect(() => {
    if (summaryGeneratedRef.current) return;
    const hasFinalized = blocks.some((b) => b.status === "completed" || b.status === "failed");
    if (!hasFinalized) return;
    const timer = setTimeout(() => {
      summaryGeneratedRef.current = true;
      summarizeCommands(blocks, sessionId, locale)
        .then((summary) => {
          if (summary) onSummaryUpdateRef.current?.(summary);
        })
        .catch(() => {});
    }, 1500);
    return () => clearTimeout(timer);
  }, [blocks, sessionId, locale]);

  // Bridge blockSearchCursor into a ref so doSearch can read the latest cursor without
  // depending on the state value itself — depending on it directly would give doSearch a
  // new identity every time a match is found (setBlockSearchCursor -> re-render -> new
  // doSearch), which re-triggers the "search as you type" effect below (it depends on
  // doSearch) and cascades through every match instead of stopping at the first.
  const blockSearchCursorRef = useRef(blockSearchCursor);
  useEffect(() => { blockSearchCursorRef.current = blockSearchCursor; }, [blockSearchCursor]);

  // Auto-scroll the completed-block list to the bottom whenever a new card becomes visible.
  const visibleBlockCount = blocks.filter((b) => b.status !== "running" && b.renderedLines).length;
  useEffect(() => {
    blockListRef.current?.scrollTo({ top: blockListRef.current.scrollHeight });
  }, [visibleBlockCount]);

  // The terminal wrapper is `display:none` while viewTab === "files" (see the
  // JSX below). xterm.js can receive writes (e.g. the shell's own prompt
  // redraw after a `cd` finishes) while hidden without actually painting them
  // — canvas-based rendering can't measure a zero-size/hidden element — so
  // switching back to the Terminal tab can show a blank live pane until some
  // other interaction (typing) forces a repaint. Force one explicitly here.
  useEffect(() => {
    if (viewTab !== "terminal") return;
    const term = termRef.current;
    if (!term) return;
    fitAddonRef.current?.fit();
    term.refresh(0, term.rows - 1);
  }, [viewTab]);

  // The live terminal pane visually shrinks to just MIN_LIVE_ROWS while idle
  // (just a prompt, nothing running) instead of always reserving a big fixed
  // box, and snaps straight to MAX_LIVE_ROWS the instant any real PTY output
  // arrives (see the onPtyData handler below) — deliberately binary rather
  // than trying to track "how many rows are actually needed": that was tried
  // via cursor-position tracking and broke for TUI-style content (interactive
  // menus/prompts that reposition the cursor non-sequentially to redraw
  // specific lines), leaving the pane stuck too small with no way to scroll
  // into view since mouse-wheel scroll has no connection to this state.
  // Resets back to MIN_LIVE_ROWS whenever the visible block count changes,
  // since that only happens right after useTerminalBlocks clears the live
  // terminal (a real command finished) or wipes it (the `clear` command) —
  // either way the live pane is freshly empty at that point.
  const [liveRows, setLiveRows] = useState(MIN_LIVE_ROWS);
  useEffect(() => {
    // Every platform shrinks here, Windows included.
    //
    // Windows used to skip this, to work around a real-machine bug where a
    // slow custom prompt (oh-my-posh) printed its prompt text only after the
    // block had already flipped to "completed" — too late for the
    // MAX_LIVE_ROWS bump below, which is deliberately gated on a block being
    // "running" — leaving the prompt clipped out of a pane already shrunk to
    // 3 rows. That workaround is now both unnecessary and harmful: the pane
    // starts at the prompt's own row on Windows (see liveTopRows), so the
    // prompt is always the first visible row however late it arrives. And
    // since Windows no longer clears the xterm buffer (see useTerminalBlocks'
    // OSC 133 D branch), a pane left expanded would keep showing the old
    // output that already has a card.
    setLiveRows(MIN_LIVE_ROWS);
  }, [visibleBlockCount]);

  // How many rows to scroll the xterm host up by, so the live pane's first
  // visible row is the prompt's own row.
  //
  // Windows only, and only because the buffer is never cleared there (see
  // useTerminalBlocks' OSC 133 D branch): the whole ConPTY screen is present,
  // so "show from the top" would show the oldest rows. A fixed bottom
  // anchor was tried first and is wrong in the opposite direction — it
  // assumes the prompt is on the last row, which is only true once the
  // screen has filled up. On a freshly-opened tab the prompt is on row 1
  // with blank rows beneath it, and bottom-anchoring rendered an entirely
  // blank pane (reported from a real machine). Tracking the prompt's actual
  // row is the only thing correct in both states. Other platforms clear the
  // buffer, so their prompt is always at row 0 and this stays 0.
  const [liveTopRows, setLiveTopRows] = useState(0);
  const syncLiveTop = useCallback(() => {
    if (!navigator.platform.toLowerCase().startsWith("win")) return;
    const term = termRef.current;
    const promptAbsRow = promptAbsRowRef.current;
    if (!term || promptAbsRow === null) return;
    const viewportRow = promptAbsRow - term.buffer.active.baseY;
    setLiveTopRows(Math.max(0, Math.min(term.rows - 1, viewportRow)));
  }, []);
  syncLiveTopRef.current = syncLiveTop;

  // 見上面 untrackedCommandBoundaryRef 宣告處的說明——這裡才真的賦值，
  // 因為 setLiveRows 要到這裡才存在。跟這個檔案其他 ref 一樣直接在
  // render 當下賦值，不用額外包一層 effect。這整段只在指令文字還原失敗
  // 的保底情況下才會被呼叫，正常情況下走 beginTrackedBlock 直接變成卡片，
  // 不會執行到這裡。
  //
  // "start" 先撐到 MAX_LIVE_ROWS，避免指令執行途中輸出被裁掉（滑鼠滾輪
  // 跟 liveRows 毫無關聯，裁掉了就拿不回來）。"end" 則改成量測游標實際
  // 停在第幾行，把 liveRows 收回剛好能放下這次輸出的高度，而不是繼續
  // 卡在 MAX——本機自己的指令結束後内容會被 finalizeBlock 搬進卡片、
  // 現場清空，收回 MIN_LIVE_ROWS 沒有問題；但遠端觀看者送進來的指令
  // 永遠不會變成卡片、也永遠不會被清空，維持在 MAX 只會在輸出行數本來
  // 就不多時，於實際內容下方留一大截用不到的空白，跟原本終端機的樣子
  // 不一致（實機截圖證實）。這裡只在 "end" 當下讀一次游標位置，不是
  // 持續追蹤——不會遇到上面那段中文註解講的、追蹤游標位置在互動式全螢幕
  // 內容（TUI）上失準的問題，因為那類內容早就走 isAlternateBuffer 的
  // 高度計算，不受這裡影響。
  untrackedCommandBoundaryRef.current = (kind) => {
    if (kind === "start") {
      setLiveRows(MAX_LIVE_ROWS);
      return;
    }
    const term = termRef.current;
    const usedRows = term ? term.buffer.active.cursorY + 1 : MIN_LIVE_ROWS;
    setLiveRows(Math.min(MAX_LIVE_ROWS, Math.max(MIN_LIVE_ROWS, usedRows)));
  };

  // Agent lifecycle status shown in the AgentStatusBar above the input, driven by
  // runAgentLoop/handleAiQuery via the onPhase callback (see handleAgentPhase).
  const [agentPhase, setAgentPhase] = useState<AgentPhase | null>(null);
  const agentStepRef = useRef(0);
  const handleAgentPhase = useCallback((update: AgentPhase) => {
    // Track the count of steps that actually did work (ran a command / did web),
    // NOT the "asking" iterations — the final iteration is just the AI confirming
    // DONE and executes nothing, so counting it would inflate the "done (N steps)"
    // total (e.g. a one-command task would misreport as 2).
    if (update.phase === "running" || update.phase === "web") {
      agentStepRef.current = update.step;
    }
    setAgentPhase(update);
  }, []);

  // xterm doesn't expose cell height as public API — this reads the same internal
  // renderer field the (now-removed) old block overlay used, with a font-metrics
  // fallback for the first render or if that internal ever changes shape.
  const cellHeightPx =
    (termState as unknown as { _core?: { _renderService?: { dimensions?: { css?: { cell?: { height?: number } } } } } } | null)
      ?._core?._renderService?.dimensions?.css?.cell?.height || 14 * 1.1;
  const liveHeightPx = Math.round(liveRows * cellHeightPx);

  // How far to slide the xterm host up so the prompt is the live pane's first
  // visible row — see liveTopRows and the host element's style comment. Always
  // 0 while a full-screen TUI owns the alternate buffer: that draws its own
  // complete screen into a full-height frame, with nothing to clip or offset.
  //
  // TERMINAL_HOST_PADDING_PX must be added on top of the row arithmetic:
  // .aiterm-terminal-root carries `padding: 4px`, so row N's text actually
  // starts at `4 + N * cellHeight` inside the host. Sliding by only
  // `N * cellHeight` leaves row N-1's bottom 4px sitting in the frame's top
  // 4px — visible as a thin half-cut strip of already-carded output above the
  // prompt (confirmed from a real-machine screenshot). Rounded up rather than
  // to nearest so a fractional cell height can only ever over-shift by a
  // sub-pixel (imperceptibly trimming the prompt row's own top) instead of
  // under-shifting back into that same sliver.
  const liveTopOffsetPx =
    isAlternateBuffer || liveTopRows <= 0
      ? 0
      : Math.ceil(liveTopRows * cellHeightPx) + TERMINAL_HOST_PADDING_PX;

  // Fetch git info (branch, insertions/deletions) for completed blocks, debounced 500ms.
  const gitFetchTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    const pending = blocks.find((b) => b.status !== "running" && b.gitInfo === undefined && b.cwd);
    if (!pending) return;
    if (gitFetchTimerRef.current) return;

    gitFetchTimerRef.current = setTimeout(async () => {
      gitFetchTimerRef.current = null;
      const stillPending = blocksRef.current.filter((b) => b.status !== "running" && b.gitInfo === undefined && b.cwd);
      for (const b of stillPending) {
        try {
          const info = await getGitBlockInfo(b.cwd!);
          setBlockGitInfo(b.id, info);
        } catch {
          setBlockGitInfo(b.id, null);
        }
      }
    }, 500);
  }, [blocks]);

  // Apply font changes from Settings
  useEffect(() => {
    const handler = (e: Event) => {
      const { fontSize, fontFamily } = (e as CustomEvent).detail as { fontSize: number; fontFamily: string };
      const term = termRef.current;
      if (!term) return;
      term.options.fontSize = fontSize;
      term.options.fontFamily = fontFamily;
      const fit = fitAddonRef.current;
      if (fit) requestAnimationFrame(() => fit.fit());
    };
    window.addEventListener("aiterm:font-changed", handler);
    return () => window.removeEventListener("aiterm:font-changed", handler);
  }, []);

  // Apply theme changes from Settings
  useEffect(() => {
    const handler = (e: Event) => {
      const { theme } = (e as CustomEvent).detail as { theme: AppTheme };
      const term = termRef.current;
      if (!term) return;
      term.options.theme = theme.xterm;
    };
    window.addEventListener("aiterm:theme-changed", handler);
    return () => window.removeEventListener("aiterm:theme-changed", handler);
  }, []);

  // File paste/drop → write absolute path to PTY
  useEffect(() => {
    const el = hostRef.current;
    if (!el) return;

    const handlePaste = async (e: ClipboardEvent) => {
      const files = e.clipboardData?.files;
      if (!files || files.length === 0) return; // let xterm handle text paste
      e.preventDefault();
      e.stopPropagation();
      const sid = sessionRef.current;
      if (!sid) return;
      // A clipboard-pasted File has no usable filesystem path (unlike OS
      // drag-and-drop) — materialize its bytes to a real temp file so the
      // program in the PTY has something it can actually open.
      const paths = (await Promise.all(
        Array.from(files).map((f) => resolvePastedFilePath(f))
      )).join(" ");
      // Wrap in the same bracketed-paste sequence xterm.js uses for genuine
      // text pastes (ESC[200~...ESC[201~), so a program like Claude Code CLI
      // can tell this arrived as one pasted unit rather than typed
      // characters — that's what lets it render a "[Image #1]" attachment
      // chip instead of echoing the raw path.
      const bracketed = termRef.current?.modes.bracketedPasteMode
        ? "\x1b[200~" + paths + "\x1b[201~"
        : paths;
      await writePty(sid, bracketed).catch(() => {});
    };

    const handleDragOver = (e: DragEvent) => {
      e.preventDefault();
    };

    const handleDrop = async (e: DragEvent) => {
      e.preventDefault();
      const files = e.dataTransfer?.files;
      if (!files || files.length === 0) return;
      const sid = sessionRef.current;
      if (!sid) return;
      const paths = Array.from(files)
        .map((f) => (f as File & { path?: string }).path ?? f.name)
        .join(" ");
      await writePty(sid, paths + " ").catch(() => {});
    };

    // Capture phase: xterm.js's own paste listener lives on its textarea (the
    // event target) and unconditionally calls stopPropagation() regardless of
    // whether the clipboard held text or files, which would otherwise stop
    // this handler — registered on an ancestor — from ever seeing file/image
    // pastes at all.
    el.addEventListener("paste", handlePaste, true);
    el.addEventListener("dragover", handleDragOver);
    el.addEventListener("drop", handleDrop);
    return () => {
      el.removeEventListener("paste", handlePaste, true);
      el.removeEventListener("dragover", handleDragOver);
      el.removeEventListener("drop", handleDrop);
    };
  }, []); // hostRef and sessionRef are stable refs — no deps needed

  // Telegram Remote Control
  // ownerKey 用 tab.id，不是 sessionId：sessionId 在 PTY 建立前是空字串，
  // 空字串跟 remoteOwner 的初始值 null 比較沒有意義；tab.id 是穩定的 UUID。
  const { isRemoteEnabled, toggleRemote, sendRemoteResponse } = useTelegramRemoteControl(
    tabId,
    remoteOwner,
    (owner) => onRemoteOwnerChange?.(owner),
    (text) => {
      const agentQuery = parseAgentPrefix(text);
      const aiQuery = parseAiPrefix(text);
      if (agentQuery !== null || aiQuery !== null) {
        const finalQuery = agentQuery || aiQuery!;
        if (previewRef.current.loading) return;
        agentAbortRef.current = false;
        startMission(finalQuery, 5);
        if (sessionRef.current && termRef.current) {
          runAgentLoop({
            t,
            goal: finalQuery,
            queryFn: (q) => invokeAiQuery(q, sessionRef.current!, locale),
            term: termRef.current,
            getSubmitCommand: () => submitCommandRef.current,
            setPreview,
            setStreamText,
            streamingRef,
            executionModeRef,
            writeRed: (msg) => termRef.current?.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n`),
            abortRef: agentAbortRef,
            stepCount: 0,
            maxSteps: maxAgentStepsRef.current,
            history: [],
            onPhase: handleAgentPhase,
            onComplete: (explanation?: string) => {
              setAgentPhase({ phase: "done", steps: agentStepRef.current });
              stopMission();
              if (sessionRef.current) writePty(sessionRef.current, "\r").catch(console.error);
              sendRemoteResponse(explanation ? `Agent: ${explanation}` : "[Agent Mission Completed] 🎉");
              onMissionEnd?.();
            },
            onFail: (msg) => {
              setAgentPhase({ phase: "failed", reason: msg });
              stopMission();
              if (sessionRef.current) writePty(sessionRef.current, "\r").catch(console.error);
              sendRemoteResponse(`⚠ Agent stopped: ${msg}`);
              onMissionEnd?.();
            },
            onStepComplete: (info: AgentStepInfo) => reportAgentStep(info, { sendRemoteResponse, onAgentProgress }),
          });
        }
      } else {
        submitCommand(text, (block) => {
          let output = block.rawOutput.trim();
          if (output.length > 0) {
            if (output.length > 4000) {
              output = output.substring(0, 4000) + "\n... (output truncated)";
            }
            sendRemoteResponse(output);
          } else {
            sendRemoteResponse(`(Command finished: ${block.command})`);
          }
        });
      }
    }
  );

  /** Fetch active provider name and execution mode from config. */
  function refreshConfig() {
    listProviders()
      .then((list) => {
        const active = list.find((p) => p.is_default) ?? list[0];
        setActiveProvider(active?.display_name ?? "");
        setActiveProviderId(active?.id ?? "");
      })
      .catch(() => {});

    getConfig()
      .then((cfg) => {
        executionModeRef.current = cfg.execution_mode;
        setSubmitShortcutState(cfg.submit_shortcut);
        // 0 means unlimited — use a very large number internally
        maxAgentStepsRef.current = cfg.max_agent_steps === 0 ? 9999 : (cfg.max_agent_steps ?? 5);
      })
      .catch(() => {});
  }

  useEffect(() => {
    refreshConfig();
  }, []);

  // Re-fetch config when returning from settings.
  useEffect(() => {
    const handler = () => {
      if (document.visibilityState === "visible") refreshConfig();
    };
    document.addEventListener("visibilitychange", handler);
    return () => document.removeEventListener("visibilitychange", handler);
  }, []);

  // Keyboard shortcuts: Ctrl+, → settings, Ctrl+Shift+P → palette
  useEffect(() => {
    if (!isActive) return;
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === ",") {
        e.preventDefault();
        navigate("/settings");
      } else if (e.ctrlKey && e.shiftKey && e.key === "R") {
        e.preventDefault();
        setBookmarksOpen(true);
      } else if (e.ctrlKey && e.shiftKey && e.key === "P") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      } else if (e.ctrlKey && (e.key === "i" || e.key === "I")) {
        e.preventDefault();
        setPanelOpen((o) => !o);
      } else if (e.ctrlKey && e.key === "f") {
        e.preventDefault();
        setSearchOpen(true);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [navigate, isActive]);

  // Listen for "Ask AI" clicks from block action buttons
  useEffect(() => {
    if (!isActive) return;
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail as { command?: string; exitCode?: number };
      setPanelOpen(true);
      
      if (detail && detail.command) {
        // Dispatch another event that AiPanel can pick up to pre-fill context
        setTimeout(() => {
          window.dispatchEvent(new CustomEvent("aiterm:prefill-chat", {
            detail: { text: `指令 \`${detail.command}\` 執行失敗 (exit code ${detail.exitCode})。請分析可能原因並提供修復建議。` }
          }));
        }, 100);
      }
    };
    window.addEventListener("aiterm:ask-ai", handler);
    return () => window.removeEventListener("aiterm:ask-ai", handler);
  }, [isActive]);

  // (Agent loop is now callback-driven via runAgentLoop — no useEffect needed)

  // 分頁一建立就帶著任務時，自動啟動 agent loop。兩個來源：企業任務派送，
  // 以及首頁輸入框（那個沒有 enterpriseTask）。底下 onComplete/onFail 裡的
  // git push/PR 與 enterpriseCompleteTask 都包在 enterpriseTask 判斷內，
  // 所以非企業來源的任務會正常跑完、正常清進度，只是跳過企業收尾。
  const initialMissionFiredRef = useRef(false);
  useEffect(() => {
    if (!initialMission || initialMissionFiredRef.current) return;
    if (!sessionId || !termRef.current) return;
    initialMissionFiredRef.current = true;
    const term = termRef.current;
    const session = sessionId;
    agentAbortRef.current = false;
    startMission(initialMission.goal, initialMission.maxSteps);
    // Brief delay to let the shell finish initializing before the first AI query.
    setTimeout(() => {
      runAgentLoop({
        t,
        goal: initialMission.goal,
        queryFn: (q) => invokeAiQuery(q, session, locale),
        term,
        getSubmitCommand: () => submitCommandRef.current,
        setPreview,
        setStreamText,
        streamingRef,
        executionModeRef,
        writeRed: (msg) => term.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n`),
        abortRef: agentAbortRef,
        stepCount: 0,
        maxSteps: initialMission.maxSteps,
        history: [],
        onStepComplete: ({ stepIndex, maxSteps: total }) => {
          onAgentProgress?.(stepIndex, total);
        },
        onComplete: () => {
          term.write(`\r\n\x1b[32m[Enterprise Task Completed] ✓\x1b[0m\r\n`);
          stopMission();
          writePty(session, "\r").catch(console.error);
          onMissionEnd?.();
          // Trigger on_complete (push + optional PR) and mark task done.
          if (enterpriseTask && initialCwd) {
            term.write(`\r\n\x1b[36m[Enterprise: running on_complete...]\x1b[0m\r\n`);
            enterpriseOnComplete({
              taskId: enterpriseTask.taskId,
              repoDir: initialCwd,
              workBranch: enterpriseTask.workBranch,
              onComplete: enterpriseTask.onComplete,
            }).then((msg) => {
              term.write(`\r\n\x1b[32m${msg}\x1b[0m\r\n`);
              enterpriseCompleteTask(enterpriseTask.taskId).catch(console.error);
            }).catch((err) => {
              term.write(`\r\n\x1b[31m[on_complete error: ${err}]\x1b[0m\r\n`);
              enterpriseCompleteTask(enterpriseTask.taskId).catch(console.error);
            });
          }
        },
        onFail: (msg) => {
          term.write(`\r\n\x1b[33m⚠ Enterprise Task stopped: ${msg}\x1b[0m\r\n`);
          stopMission();
          writePty(session, "\r").catch(console.error);
          onMissionEnd?.();
          if (enterpriseTask) {
            enterpriseCompleteTask(enterpriseTask.taskId).catch(console.error);
          }
        },
      });
    }, 1500);
  }, [sessionId, initialMission, startMission, stopMission, enterpriseTask, initialCwd]);

  useEffect(() => {
    if (!hostRef.current) return;

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
    termRef.current = term;
    setTermState(term);

    // xterm.js maps Ctrl+<letter> to the matching Unix control byte (Ctrl+V →
    // \x16, SYN) and sends it straight to the pty, which is correct terminal
    // behavior but means it also swallows the keydown (preventDefault +
    // stopPropagation) before the browser's native paste ever runs — so
    // Ctrl+V does nothing on Windows/Linux where users expect it to paste.
    // macOS is unaffected since paste there is Cmd+V (metaKey), which never
    // enters this path. Returning false here tells xterm.js to skip its own
    // handling for this one combination and let the browser's default paste
    // action proceed instead.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type === "keydown" && e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey && e.key === "v") {
        return false;
      }
      return true;
    });

    const fit = new FitAddon();
    term.loadAddon(fit);
    fitAddonRef.current = fit;
    const searchAddon = new SearchAddon();
    term.loadAddon(searchAddon);
    searchAddonRef.current = searchAddon;
    term.open(hostRef.current);
    requestAnimationFrame(() => fit.fit());

    // Pin the live frame's internal scroll to the top. The frame is
    // overflow:hidden but its xterm host is a fixed 220px (taller than the
    // frame's visible height while idle), so the browser CAN still scroll the
    // frame internally — notably it scrolls ancestors to reveal xterm's focused
    // helper-textarea after a paste. That pushed the actual content (row 0, the
    // live prompt + typed/pasted command) up out of the clipped viewport,
    // leaving the pane looking blank even though the buffer and DOM were
    // perfectly correct. Root-caused via geometry logging: on the second paste
    // the frame's scrollTop jumped to ~170px, offsetting the host's row 0 that
    // far above the visible window. The live pane always renders from the top
    // (xterm handles content scrolling internally within its own rows), so any
    // non-zero frame scroll is spurious — snap it back. The `!== 0` guards keep
    // the corrective assignment from firing a second scroll event and looping.
    const liveFrame = hostRef.current.parentElement;
    const pinLiveFrameScroll = () => {
      if (!liveFrame) return;
      if (liveFrame.scrollTop !== 0) liveFrame.scrollTop = 0;
      if (liveFrame.scrollLeft !== 0) liveFrame.scrollLeft = 0;
    };
    liveFrame?.addEventListener("scroll", pinLiveFrameScroll);

    // The live pane shows "what's happening right now"; scrolling back
    // through history is what the block cards above are for. Since the xterm
    // buffer is no longer cleared on Windows (see useTerminalBlocks' OSC 133 D
    // branch), the wheel would otherwise scroll into scrollback and re-reveal
    // the very output that already has a card sitting above it.
    //
    // Not simply swallowed: a wheel gesture that does nothing at all reads as
    // a frozen UI. preventDefault stops xterm's own viewport from scrolling,
    // then the delta is forwarded to the real scroll container (the block
    // list), so the wheel behaves the same over the live pane as it does over
    // any card.
    //
    // Alternate buffer is deliberately exempt: full-screen programs (vim,
    // htop, less) either consume the wheel themselves or have xterm translate
    // it into key/mouse events for them, and they own the whole frame anyway
    // — there is no card list to scroll instead.
    const onLiveWheel = (e: WheelEvent) => {
      if (isAlternateBufferRef.current) return;
      e.preventDefault();
      const scroller = blockListRef.current;
      if (scroller) scroller.scrollTop += e.deltaY;
    };
    // Non-passive: a passive listener cannot preventDefault, and Chromium
    // treats wheel listeners as passive by default.
    hostRef.current.addEventListener("wheel", onLiveWheel, { passive: false });
    const liveWheelHost = hostRef.current;

    // 注音/中文組字進行中就不要強制重繪（見下方寫入路徑的 isWindows 分支）。
    //
    // 那個強制 refresh 是 Windows/WebView2 專屬的補償，用來避免直接打進即時
    // 終端的字沒被畫出來。但它會打斷瀏覽器正在合成的 IME 字串——同一個衝突
    // 早就在 macOS 上被發現過，所以 macOS 分支才不做這件事。
    //
    // 一般 shell 不會撞到，因為你打字的當下沒有輸出進來；但持續重繪的 TUI
    // （Claude Code）每秒吐好幾塊，組字期間必然對撞。實測確認：同樣的環境
    // 變數在一般終端機分頁也重現，所以與分頁類型無關，就是輸出頻率。
    const term$ = term as unknown as { textarea?: HTMLTextAreaElement };
    const ta = term$.textarea;
    const onCompositionStart = () => { isComposingRef.current = true; };
    const onCompositionEnd = () => {
      isComposingRef.current = false;
      // 組字期間跳過的重繪在這裡一次補上，否則那段輸出可能停在沒畫出來的狀態。
      termRef.current?.refresh(0, (termRef.current.rows ?? 1) - 1);
    };
    ta?.addEventListener("compositionstart", onCompositionStart);
    ta?.addEventListener("compositionend", onCompositionEnd);

    // DECTCEM（ESC[?25h / ESC[?25l）。回傳 false 代表「我只是旁聽」，xterm 自己
    // 的 handler 仍會照常執行。用 parser handler 而不是自己掃位元組，是因為序列
    // 可能被 PTY chunk 邊界切開，字串比對會漏。
    // ?25 只是眾多 DEC private mode 之一（?1049、?2004… 都會走進這裡）。
    //
    // ESC[?25h 一到就立刻採信：那是應用程式在說「游標現在在該在的地方」。
    // 反方向才去抖：實測（pty 錄下 Claude Code 的原始位元組）每一幀都是
    //   ESC[?25l → 重畫 → ESC[<row>;<col>H → ESC[?25h
    // vim 之類也是同一套，所以「藏了一下下」是 TUI 正常的重繪行為，不代表游標
    // 不可信；連續藏著超過門檻才算真的把游標交出去不管了。
    //
    // 注意：這個旗標**不足以**判斷該不該釘 IME 位置。實測 macOS 閒置時同樣停在
    // ESC[?25l，兩個平台都是隱藏——真正的判準是找不找得到自繪 caret，見 appCaret。
    const HIDDEN_SETTLE_MS = 500;
    let visibilityTimer: ReturnType<typeof setTimeout> | null = null;
    let rawHidden = false;
    const trackCursorVisibility = (visible: boolean) => (params: (number | number[])[]) => {
      if (params.some((p) => p === 25) && rawHidden === visible) {
        rawHidden = !visible;
        if (visibilityTimer) clearTimeout(visibilityTimer);
        visibilityTimer = null;
        if (visible) setCursorHidden(false);
        else visibilityTimer = setTimeout(() => setCursorHidden(true), HIDDEN_SETTLE_MS);
      }
      return false;
    };

    const decSet = term.parser.registerCsiHandler({ prefix: "?", final: "h" }, trackCursorVisibility(true));
    const decReset = term.parser.registerCsiHandler({ prefix: "?", final: "l" }, trackCursorVisibility(false));

    const decoder = new TextDecoder("utf-8");

    let unlistenData: (() => void) | null = null;
    let unlistenStream: Promise<() => void> | null = null;
    let cancelled = false;
    // Set the moment the live onPtyData listener below writes its first real
    // chunk — read by the backfill check further down to avoid double-
    // printing content the live path already handled. See that check's
    // comment for why backfilling is needed at all.
    let hasReceivedLiveChunk = false;

    const writeRed = (msg: string) => {
      term.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n`);
    };

    (async () => {
      try {
        const { rows, cols } = term;
        let id: string;
        if (externalSessionId) {
          // Backend already has a live PTY session for us (spawned by an MCP
          // coordination tool) — adopt it instead of creating a new one.
          id = externalSessionId;
        } else {
          const lastCwd = initialCwd ?? localStorage.getItem("aiterm_last_cwd") ?? undefined;
          id = await createPty({ rows, cols }, lastCwd, claudeBridge);
          if (cancelled) {
            // Torn down (e.g. React StrictMode's dev-mode synthetic
            // remount) before this newly-created session could be used —
            // nothing else knows about it yet, so close it now instead of
            // leaking an orphaned PTY process. Never applies to an adopted
            // (externalSessionId) session, which is owned elsewhere and
            // must never be closed from here — see the cleanup below.
            closePty(id).catch(() => {});
            return;
          }
        }
        sessionRef.current = id;
        // A real resize can land while createPty() was still in flight (no
        // session id yet to resize) — see pendingResizeRef's comment. Flush
        // it now that a session finally exists, same as the paste-guard
        // case does on Enter/timeout.
        if (pendingResizeRef.current) {
          const pending = pendingResizeRef.current;
          pendingResizeRef.current = null;
          resizePty(id, pending).catch(console.error);
        }
        setSessionId(id);
        onSessionCreated?.(id);
        setStatus(`connected (${id.slice(0, 8)}…)`);

        const isWindows = navigator.platform.toLowerCase().startsWith("win");

        const unlisten = await onPtyData(id, (bytes) => {
          lastPtyOutputAtRef.current = Date.now();
          const text = decoder.decode(bytes, { stream: true });
          hasReceivedLiveChunk = true;

          // 實機測試抓到的 bug：appendOutput(text) 原本在 term.write(text)
          // 呼叫「之後」就同步執行，隱含假設這個 chunk 已經被 xterm 解析
          // 完畢——但 @xterm/xterm 的 WriteBuffer.write()（見
          // node_modules/@xterm/xterm/src/common/input/WriteBuffer.ts）對
          // 不是緊跟著「使用者剛輸入」的資料，一律用 setTimeout 排到下一輪
          // 事件迴圈才真正解析，不是呼叫當下就同步跑完。單一小 chunk 通常
          // 沒事；但像 ifconfig 這種一次湧入一大串 chunk 的情況，好幾個
          // chunk 會搶在 xterm 排定的那次非同步解析之前，就把
          // appendOutput 全部呼叫完——這時候 OSC 133 C 的 handler（掛在
          // useTerminalBlocks.ts 的 recoverUntrackedCommand/
          // beginTrackedBlock 上，用來讓遠端指令變成卡片）根本還沒被觸發、
          // 區塊還不存在，appendOutput 找不到任何 running 中的區塊可以
          // 附加內容，這些輸出就直接被略過、永遠救不回來；等 xterm 終於
          // 追上、C 跟緊接在後的 D 幾乎同時處理完畢，畫面上就會看到一張
          // 執行時間近乎 0ms、內容整個消失、只剩指令文字的空卡片。
          // 改成把 appendOutput 以及依賴 blocksRef 的 liveRows 判斷都搬進
          // term.write() 的完成 callback，保證同一個 chunk 的 OSC 標記
          // 一定已經先被處理過。
          // Routed through resizeRepaintGateRef so a burst of ConPTY
          // resize-triggered full-screen repaints (see resizeRepaintGate.ts)
          // collapses to just the last one instead of flashing stale
          // scrollback content — everything below operates on whatever
          // chunk the gate actually decides to write, not necessarily the
          // raw `text` that arrived.
          const writeChunk = (chunkText: string) => {
            const onWriteComplete = () => {
              appendOutput(chunkText);
              // Snap the live pane straight to full height the moment a tracked
              // command is actually running and producing output, rather than
              // trying to precisely track how many rows are "needed" via cursor
              // position — that broke for TUI-style content (interactive menus,
              // prompts) that reposition the cursor non-sequentially to redraw
              // specific lines, leaving liveRows stuck too small with no way to
              // scroll into view (mouse-wheel scroll has no connection to
              // liveRows at all). Gated on a block actually being "running" —
              // not just "any PTY data arrived" — so the shell's own idle-prompt
              // output (on connect, or after a command finishes) doesn't also
              // trigger this: that data isn't part of any tracked block, so
              // there'd be no `visibleBlockCount` change afterward to shrink it
              // back down again, leaving the pane stuck at full height forever.
              const latestBlock = blocksRef.current[blocksRef.current.length - 1];
              if (latestBlock?.status === "running") {
                setLiveRows(MAX_LIVE_ROWS);
              }
              // The prompt's viewport-relative row moves whenever output
              // scrolls the buffer (baseY grows), so the offset has to be
              // recomputed per chunk, not just when OSC 133 B fires. No-op
              // off Windows.
              syncLiveTop();
            };

            if (isWindows) {
              // Force a repaint once xterm has actually finished processing this
              // chunk (the write() completion callback, not just the write() call
              // returning — writes are queued/async internally). Without this,
              // characters typed directly into the live terminal (echoed back by
              // the shell through the PTY, not rendered locally like WarpInput)
              // can silently fail to paint on some Windows/WebView2 setups even
              // though they were received and buffered correctly — the dropped
              // keystrokes still reach the shell (confirmed by Enter submitting
              // the right command), only the on-screen echo goes missing.
              // Windows-only: forcing this on every chunk on macOS turned out to
              // corrupt in-progress IME composition (e.g. Zhuyin/Bopomofo input)
              // in the live pane, which never needed this — macOS renders every
              // write reliably on its own.
              // 組字進行中一律走普通寫入——強制 refresh 會打斷 IME 合成，
              // 讓組字中的字串跑到畫面別處。compositionend 會補一次重繪。
              if (isComposingRef.current) {
                term.write(chunkText, onWriteComplete);
              } else {
                term.write(chunkText, () => {
                  term.refresh(0, term.rows - 1);
                  onWriteComplete();
                });
              }
            } else {
              term.write(chunkText, onWriteComplete);
            }
          };

          resizeRepaintGateRef.current!.handleChunk(text, isAlternateBufferRef.current, writeChunk);

          // Detect password prompts during agent mode
          if (agentMissionRef.current?.active) {
            const lower = text.toLowerCase();
            if (lower.includes("password") || lower.includes("密碼") || lower.includes("passphrase")) {
              term.write(`\r\n\x1b[33;1m${t.term_agent_wait_password}\x1b[0m\r\n`);
            }
          }
        });
        if (cancelled) {
          // Same race as above, but past the listener-registration await:
          // this component's cleanup already ran (and can never run again
          // for this mount) before the subscription finished setting up.
          // Tear it down immediately instead of leaving a listener
          // permanently subscribed to this session's real PTY output —
          // otherwise a StrictMode-orphaned mount keeps double-processing
          // every future chunk for the life of the session.
          unlisten();
          return;
        }
        unlistenData = unlisten;

        // Any session can have real output — a shell's own first prompt
        // draw, an adopted coordination tab's startup banner, even an
        // agent's first reply — that streamed out on pty://data/{id} before
        // this listener's registration IPC round-trip completed. Tauri
        // events are fire-and-forget; there's no buffering for a late
        // subscriber. For adopted (externalSessionId) sessions this race is
        // essentially always lost (the backend creates the session, and may
        // write an initial command, well before the frontend even learns a
        // tab exists); for freshly-created ones it's intermittent — a shell
        // can draw its prompt faster than this subscribe round-trip
        // completes, especially under load, leaving a freshly-opened tab
        // looking permanently blank until some later PTY write (e.g. a
        // resize-triggered shell redraw) finally arrives. Backfill from
        // PtyManager's per-session ring buffer so a tab never opens to a
        // misleadingly blank screen either way. This is ANSI-stripped plain
        // text (get_recent_output strips escape codes for AI-context use),
        // so it won't look pixel-identical to a live xterm stream — good
        // enough to "catch up," not meant to replace real-time rendering.
        // Guarded on hasReceivedLiveChunk to avoid double-printing output
        // the live listener above already wrote in the (much narrower)
        // window between subscribing and this backfill check completing.
        const backfill = await getPtyRecentOutput(id);
        if (cancelled || hasReceivedLiveChunk) return;
        if (backfill) {
          // Root-caused via diagnostic logging: xterm's own initial fit()
          // call (right after term.open()) resizes from the default 80x24
          // to the container's real size, and that resize is forwarded to
          // the backend PTY — which, like any real terminal resize, makes
          // the shell redraw its prompt in place (SIGWINCH). That redraw
          // lands in the ring buffer right alongside the shell's own
          // original connect-time draw. Since a redraw overwrites the same
          // line (carriage return, not newline) and get_recent_output
          // strips ANSI/control bytes for AI-context use, the two draws
          // arrive back-to-back with no separator at all — not as two
          // \n-delimited lines — so detect "the whole string is the same
          // chunk repeated" directly rather than assuming any particular
          // separator, and collapse it to one copy.
          const deduped = collapseWholeStringRepeat(backfill);
          if (externalSessionId) {
            // Coordination tabs: separate the "you missed this" block from
            // whatever real-time content follows with a trailing blank
            // line — there's always more after it (the spawned agent's own
            // banner/reply).
            term.write(`\r\n\x1b[2m${t.term_coordination_backfill_marker}\x1b[0m\r\n`);
            term.write(deduped.replace(/\n/g, "\r\n"));
            term.write(`\r\n`);
          } else {
            // A freshly-created tab's backfill is typically just the
            // shell's own prompt, still awaiting input — not a completed
            // line. Writing a trailing \r\n here would push xterm's cursor
            // onto a blank line below the prompt instead of leaving it
            // right after the prompt text, where the shell's real cursor
            // actually is.
            term.write(deduped.replace(/\n/g, "\r\n"));
          }
        }
      } catch (err) {
        console.error("Failed to create pty", err);
      }
    })();

    unlistenStream = listen<AiStreamEvent>("ai-stream", (event) => {
      if (event.payload.kind !== "query") return;
      if (event.payload.session_id !== sessionRef.current) return;
      if (!event.payload.done) {
        setStreamText((t) => t + event.payload.delta);
      }
      if (event.payload.tokens) {
        addTokens(event.payload.tokens);
      }
    });

    // Sends a resize that was held back while hasUnsubmittedPasteRef was
    // true, once it's safe to do so (see term.onResize below).
    const flushPendingResize = () => {
      const pending = pendingResizeRef.current;
      pendingResizeRef.current = null;
      if (pending && sessionRef.current) {
        // Re-arm here, not just at the original onResize firing — this can
        // fire well past the gate's original arm window (held back until
        // Enter/timeout), and it's this call, not the original resize event,
        // that actually triggers ConPTY's repaint.
        if (navigator.platform.toLowerCase().startsWith("win")) {
          resizeRepaintGateRef.current!.noteResize();
        }
        resizePty(sessionRef.current, pending).catch(console.error);
      }
    };

    const clearUnsubmittedPaste = () => {
      hasUnsubmittedPasteRef.current = false;
      if (unsubmittedPasteTimeoutRef.current) {
        clearTimeout(unsubmittedPasteTimeoutRef.current);
        unsubmittedPasteTimeoutRef.current = null;
      }
      flushPendingResize();
    };

    term.onData((data) => {
      // Panel owns keyboard while open — drop input.
      if (panelOpenRef.current) return;

      const session = sessionRef.current;
      if (!session) return;

      // Drop focus-tracking events that xterm.js emits when it loses / gains focus.
      // PSReadLine enables focus tracking (ESC[?1004h), so xterm.js sends ESC[O (focus-out)
      // or ESC[I (focus-in) when the WarpInput textarea steals focus.  Forwarding them
      // character-by-character causes PowerShell to print "[O" / "[I" as literal text.
      if (data === "\x1b[O" || data === "\x1b[I") return;

      // Snapshot the currently-rendered line (prompt + anything already
      // typed) the moment we're about to forward the first bytes of a fresh
      // input line. At Enter time we diff the ACTUAL rendered line against
      // this snapshot to recover exactly what the user typed, instead of
      // re-simulating the shell's own line editing (backspace, Ctrl+U,
      // bracketed paste, IME composition) ourselves — that kept drifting out
      // of sync with reality (corrupted IME composition, garbled/blanked
      // pastes, bracketed paste never registering a command at all).
      // Whatever's actually on screen at Enter time, minus this prefix, IS
      // the command — ground truth from the terminal itself, and identical
      // logic regardless of platform or input method. readLineExcludingInlinePrediction
      // additionally strips shells' inline "ghost" suggestions (e.g. PSReadLine
      // InlineView) so those never get captured as part of the typed text.
      if (lineStartSnapshotRef.current === null) {
        const baseY = term.buffer.active.baseY;
        lineStartSnapshotRef.current = readLineExcludingInlinePrediction(
          term.buffer.active.getLine(term.buffer.active.cursorY + baseY),
          term.buffer.active.cursorX,
        );
      }

      // Escape sequences (arrow keys, function keys, Home/End, bracketed
      // paste, etc.) must be sent as a single atomic PTY write. Iterating
      // char-by-char produces separate Tauri IPC calls with non-zero latency
      // between the ESC byte and the rest of the sequence. On Windows,
      // PSReadLine has a short escape-sequence timeout: if "[" doesn't arrive
      // fast enough after ESC, it treats ESC as a standalone keypress and
      // "[A" / "[B" / "[C" / "[D" then appear as literal text. Bracketed
      // paste (ESC[200~...ESC[201~) takes this same path with no special
      // handling needed — whatever it inserts on screen gets picked up
      // naturally by the snapshot-diff above when Enter is later pressed.
      if (data.startsWith("\x1b")) {
        writePty(session, data).catch(console.error);
        // Mark unsubmitted-paste state so term.onResize below holds back any
        // PTY resize until Enter is pressed (clearUnsubmittedPaste) or this
        // safety timeout fires — root-caused via DevTools + console logging:
        // a PTY resize while pasted-but-unsubmitted text sits on the prompt
        // line makes the shell redraw it, and that redraw can leave the
        // reverse-video/dim styling shells use for such text corrupted
        // (rendered invisible) — reproduced consistently on a second paste.
        if (data.startsWith("\x1b[200~")) {
          hasUnsubmittedPasteRef.current = true;
          if (unsubmittedPasteTimeoutRef.current) clearTimeout(unsubmittedPasteTimeoutRef.current);
          unsubmittedPasteTimeoutRef.current = setTimeout(clearUnsubmittedPaste, 5000);
        }
        return;
      }

      // Accumulate what actually gets sent to the PTY and flush it as ONE
      // atomic write at the end, instead of one unawaited writePty() call per
      // character. Per-character writes are each a separate Tauri IPC
      // round-trip with no ordering guarantee between them; invisible at
      // typing speed, but a paste delivers the whole chunk in a single
      // onData call, firing dozens of unawaited calls back-to-back with a
      // real chance of arriving at the PTY out of order.
      let toSend = "";
      // Characters from THIS chunk not yet echoed back by the PTY — the
      // rendered buffer only reflects earlier onData calls, which had time to
      // round-trip; anything typed/pasted within the current synchronous
      // chunk (e.g. a paste's own trailing Enter) hasn't painted yet.
      let pendingThisChunk = "";
      let sawNewlineThisChunk = false;
      for (const ch of data) {
        if (ch === "\r" || ch === "\n") {
          let line: string;
          if (!sawNewlineThisChunk) {
            const baseY = term.buffer.active.baseY;
            const renderedNow = readLineExcludingInlinePrediction(
              term.buffer.active.getLine(term.buffer.active.cursorY + baseY),
              term.buffer.active.cursorX,
            );
            const fullLine = renderedNow + pendingThisChunk;
            const snapshot = lineStartSnapshotRef.current ?? "";
            line = (fullLine.startsWith(snapshot) ? fullLine.slice(snapshot.length) : fullLine).trim();
          } else {
            // A later embedded line within the same chunk (multiple commands
            // pasted at once) has no real rendered prompt to diff against —
            // the shell hasn't had a chance to draw one yet — so whatever was
            // typed since the previous newline in this chunk IS the line.
            line = pendingThisChunk.trim();
          }
          sawNewlineThisChunk = true;
          pendingThisChunk = "";
          lineStartSnapshotRef.current = null;
          if (hasUnsubmittedPasteRef.current) clearUnsubmittedPaste();

          // If agent is running and user presses Enter:
          // - If a command is still running (e.g. awaiting password), pass Enter to PTY
          // - If no command is running (user typing a new command), interrupt the agent
          if (agentMissionRef.current?.active) {
             const lastBlock = blocksRef.current[blocksRef.current.length - 1];
             if (lastBlock?.status === "running") {
               // A command is running — user is probably entering a password, let it through
               toSend += ch;
               continue;
             }
             // No running command — user is trying to type a new command, interrupt agent
             agentAbortRef.current = true;
             term.write("\r\n\x1b[33m[Agent Interrupted]\x1b[0m");
             stopMission();
             // 中斷後 runAgentLoop 開頭的 abortRef 檢查會直接 return，
             // onComplete/onFail 都不會被呼叫——這裡是唯一會漏報 mission
             // 結束的出口，得自己補一次，否則首頁的進度會永遠掛著。
             onMissionEnd?.();
          }

          const agentQuery = parseAgentPrefix(line);
          const aiQuery = parseAiPrefix(line);

          if (agentQuery !== null || aiQuery !== null) {
            const finalQuery = agentQuery || aiQuery!;
            if (previewRef.current.loading) {
              writeRed("aiterm: already waiting for AI response");
              continue;
            }
            // Start Agent Loop (callback-driven, no useEffect)
            agentAbortRef.current = false;
            startMission(finalQuery, 5);
            runAgentLoop({
              t,
              goal: finalQuery,
              queryFn: (q) => invokeAiQuery(q, session, locale),
              term,
              getSubmitCommand: () => submitCommandRef.current,
              setPreview,
              setStreamText,
              streamingRef,
              executionModeRef,
              writeRed,
              abortRef: agentAbortRef,
              stepCount: 0,
              maxSteps: maxAgentStepsRef.current,
              history: [],
              onPhase: handleAgentPhase,
              onComplete: () => {
                setAgentPhase({ phase: "done", steps: agentStepRef.current });
                stopMission();
                writePty(session, "\r").catch(console.error);
                sendRemoteResponse("[Agent Mission Completed] 🎉");
                onMissionEnd?.();
              },
              onFail: (msg) => {
                setAgentPhase({ phase: "failed", reason: msg });
                stopMission();
                writePty(session, "\r").catch(console.error);
                sendRemoteResponse(`⚠ Agent stopped: ${msg}`);
                onMissionEnd?.();
              },
              onStepComplete: (info: AgentStepInfo) => reportAgentStep(info, { sendRemoteResponse, onAgentProgress }),
            });
            continue;
          }
          // Track this as a block too, same as a WarpInput-submitted command —
          // typing directly into the live terminal is a normal way to use it,
          // not a fallback path. Skip whitespace-only lines (e.g. bare Enter)
          // rather than create a block with an empty/unknown command.
          if (line) {
            beginTrackedBlockRef.current(line);
          }
          toSend += ch;
        } else if (ch === "\x7f" || ch === "\b") {
          pendingThisChunk = pendingThisChunk.slice(0, -1);
          toSend += ch;
        } else if (ch === "\x03" || ch === "\x15") {
          pendingThisChunk = "";
          toSend += ch;
        } else {
          toSend += ch;
          pendingThisChunk += ch;
        }
      }
      if (toSend) writePty(session, toSend).catch(console.error);
    });

    term.onResize(({ rows: r, cols: c }) => {
      // Arms the gate that coalesces ConPTY's resize-repaint burst — see
      // resizeRepaintGate.ts. Windows-only: no such repaint-on-resize
      // behavior exists on Unix ptys.
      if (navigator.platform.toLowerCase().startsWith("win")) {
        resizeRepaintGateRef.current!.noteResize();
      }
      // Hold this back until it's safe to send — see hasUnsubmittedPasteRef.
      if (hasUnsubmittedPasteRef.current) {
        pendingResizeRef.current = { rows: r, cols: c };
        return;
      }
      if (sessionRef.current) {
        resizePty(sessionRef.current, { rows: r, cols: c }).catch(console.error);
      } else {
        // No session yet — createPty() is still in flight. Stash it; the
        // flush right after `sessionRef.current = id` above sends it once
        // the session exists. Without this, the resize is lost for good —
        // see pendingResizeRef's comment for why nothing would ever retry it.
        pendingResizeRef.current = { rows: r, cols: c };
      }
    });

    // Debounced: a real layout change (window resize, sidebar toggle, font
    // change) can fire the ResizeObserver dozens of times in rapid
    // succession while it settles, each one calling fit() and — if the
    // measured size actually differs — firing a real PTY resize. Diagnostic
    // logging caught exactly this: a burst of 50+ resizes over ~700ms, each
    // one making the shell redraw its current prompt line, corrupting
    // whatever was on it (reproduced as pasted-but-unsubmitted text
    // rendering invisible, since the shell draws it with a reverse-video
    // escape that a mid-burst redraw could leave applied inconsistently).
    // Only the FINAL settled size actually matters to the PTY, so coalesce
    // the whole burst into one fit() call after resizing quiets down.
    let resizeSettleTimer: ReturnType<typeof setTimeout> | null = null;
    let ro: ResizeObserver | null = null;
    if (hostRef.current) {
      ro = new ResizeObserver(() => {
        if (resizeSettleTimer) clearTimeout(resizeSettleTimer);
        resizeSettleTimer = setTimeout(() => {
          fit.fit();
        }, 120);
      });
      ro.observe(hostRef.current);
    }

    // ResizeObserver only fires when the observed element's CSS-pixel
    // content-box size changes. In a remote desktop session where the
    // client's DPI scaling doesn't match the session's rendering scale
    // (DPI virtualization), a window resize can change what the user sees
    // on screen without changing the CSS-pixel dimensions WebView2 reports
    // internally — so ResizeObserver never fires, fit() never re-runs, and
    // the PTY is left at whatever size it had before, with no way to
    // recover short of restarting the session. Watching devicePixelRatio
    // itself catches exactly this case: a DPI change fires independently of
    // (and in addition to) any CSS-pixel-size change. A `resolution`
    // media query only matches once for a given DPR, so it has to be
    // re-registered after each change to keep watching.
    let dprMediaQuery: MediaQueryList | null = null;
    const watchDpr = () => {
      dprMediaQuery = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      dprMediaQuery.addEventListener("change", onDprChange, { once: true });
    };
    function onDprChange() {
      fit.fit();
      watchDpr();
    }
    watchDpr();

    return () => {
      cancelled = true;
      if (resizeSettleTimer) clearTimeout(resizeSettleTimer);
      if (unsubmittedPasteTimeoutRef.current) clearTimeout(unsubmittedPasteTimeoutRef.current);
      if (ro && hostRef.current) ro.unobserve(hostRef.current);
      dprMediaQuery?.removeEventListener("change", onDprChange);
      liveFrame?.removeEventListener("scroll", pinLiveFrameScroll);
      liveWheelHost.removeEventListener("wheel", onLiveWheel);
      ta?.removeEventListener("compositionstart", onCompositionStart);
      ta?.removeEventListener("compositionend", onCompositionEnd);
      decSet.dispose();
      decReset.dispose();
      if (visibilityTimer) clearTimeout(visibilityTimer);
      if (unlistenData) unlistenData();
      if (unlistenStream) unlistenStream.then((f: () => void) => f());
      const id = sessionRef.current;
      if (id && !externalSessionId) {
        // Adopted sessions (spawned by an MCP coordination tool, e.g.
        // spawn_tab) are NOT owned by this component — don't kill the
        // real backend PTY on unmount. This matters beyond normal
        // unmount/close: React StrictMode's dev-mode mount→cleanup→mount
        // cycle would otherwise close the one-and-only adopted session
        // during its synthetic cleanup pass, then try to re-adopt the
        // same (now-dead) session id on the second mount — the exact bug
        // this comment is here to prevent regressing. Cleanup of
        // agent-spawned sessions is intentionally out of scope for v1 —
        // see the design doc's explicit "不含 close_tab" decision.
        closePty(id).catch(() => {});
      }
      term.dispose();
      termRef.current = null;
    };
  }, []);

  const doSearch = useCallback((query: string, direction: 'next' | 'prev') => {
    if (!query) { setSearchMatchInfo(""); return; }
    const addon = searchAddonRef.current;

    // Only ever "found" via the live xterm buffer when the addon is actually
    // available — if it's momentarily unavailable, fall through to the block
    // list search below rather than reporting a false "not found".
    const foundLive = addon
      ? (direction === 'next' ? addon.findNext(query, SEARCH_OPTS) : addon.findPrevious(query, SEARCH_OPTS))
      : false;

    if (foundLive) {
      blockSearchCursorRef.current = null;
      setBlockSearchCursor(null);
      setSearchMatchInfo("found");
      return;
    }

    const match = direction === 'next'
      ? findNextBlockMatch(blocksRef.current, query, blockSearchCursorRef.current)
      : findPreviousBlockMatch(blocksRef.current, query, blockSearchCursorRef.current);

    if (match) {
      blockSearchCursorRef.current = match;
      setBlockSearchCursor(match);
      setSearchMatchInfo("found");
      requestAnimationFrame(() => {
        document.getElementById(`aiterm-block-${match.blockId}`)?.scrollIntoView({ block: "center", behavior: "smooth" });
      });
    } else {
      setSearchMatchInfo("not found");
    }
  }, []);

  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    setSearchQuery("");
    setSearchMatchInfo("");
    setBlockSearchCursor(null);
    searchAddonRef.current?.clearDecorations?.();
  }, []);

  // Focus input when search opens
  useEffect(() => {
    if (searchOpen) {
      setTimeout(() => searchInputRef.current?.focus(), 50);
    }
  }, [searchOpen]);

  // Re-run search as user types. Every query change (not just clearing it) resets the
  // block search cursor — otherwise editing a query (e.g. "e" -> "el") would resume the
  // block search from the previous query's match position, which can skip a match for the
  // new query that appears earlier in the block list. The ref is reset synchronously
  // (not just via setState) so the doSearch call below — in the same effect run — reads
  // the reset value immediately, rather than a stale one from before the ref-sync effect
  // has had a chance to run.
  useEffect(() => {
    blockSearchCursorRef.current = null;
    setBlockSearchCursor(null);
    if (searchQuery) {
      doSearch(searchQuery, 'next');
    } else {
      searchAddonRef.current?.clearDecorations?.();
      setSearchMatchInfo("");
    }
  }, [searchQuery, doSearch]);

  const handleConfirm = () => {
    if (preview.command) {
      submitCommand(preview.command);
    }
    setPreview(INITIAL_PREVIEW);
  };
  const handleCancel = () => setPreview(INITIAL_PREVIEW);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        position: "relative",
      }}
    >
      {showCloseConfirm && (
        <CloseConfirmDialog
          title={missionActiveRef.current ? t.term_close_title_mission : t.term_close_title_running}
          body={missionActiveRef.current ? t.term_close_body_mission : t.term_close_body_running}
          confirmLabel={t.term_close_discard}
          cancelLabel={t.term_close_cancel}
          onConfirm={() => handleCloseConfirm(true)}
          onCancel={() => handleCloseConfirm(false)}
        />
      )}
      <div className="aiterm-status" data-tauri-drag-region>
        <span className="aiterm-status-left" data-tauri-drag-region style={{ display: "flex", alignItems: "center", gap: "8px" }}>
          {onToggleSidebar && (
            <button
              className="aiterm-settings-btn"
              title="Toggle Sidebar (Ctrl+B)"
              onClick={onToggleSidebar}
              style={{ fontSize: "16px", padding: "0 4px", marginLeft: "-4px" }}
            >
              {isSidebarOpen ? "◧" : "☰"}
            </button>
          )}
          <span data-tauri-drag-region>AITerm · {status}</span>
        </span>
        <span style={{ display: "flex", alignItems: "center", gap: "8px" }}>
          {activeProvider ? (
            <button
              className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm aiterm-status-provider"
              title={t.term_provider_tooltip_switch}
              onClick={() => setPaletteOpen((o) => !o)}
              style={{ display: "flex", alignItems: "center", gap: "6px" }}
            >
              <RobotIcon size={14} style={{ color: "var(--accent)" }} />
              <span>{activeProvider}</span>
              {quotaWindow && <QuotaBadge window={quotaWindow} />}
            </button>
          ) : (
            <button
              className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm aiterm-status-provider aiterm-status-provider--empty"
              title={t.term_provider_tooltip_add}
              onClick={() => navigate("/settings")}
              style={{ display: "flex", alignItems: "center", gap: "6px" }}
            >
              <RobotIcon size={14} />
              <span>{t.ai_providers} ＋</span>
            </button>
          )}
          {/* **要傳 PTY session id，不是 React 的 tabId。** 後端的
              `subscribe_with_history` 拿這個值去 PtyManager 查串流；傳 React
              的分頁 id 會查不到，觀看端只會看到「那個終端機已經關閉」。
              整套自動測試都沒抓到，因為測試裡直接把 PTY id 當成 tab_id 用。 */}
          {sessionId && <SharePanel sessionId={sessionId} />}
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
            className={`aiterm-btn aiterm-btn--secondary aiterm-btn--sm ${isRemoteEnabled ? 'aiterm-agent-toggle--on' : ''}`}
            title={t.term_remote_tooltip}
            onClick={(e) => {
              e.stopPropagation();
              toggleRemote();
            }}
            style={{ display: "flex", alignItems: "center", gap: "6px" }}
          >
            <SmartphoneIcon size={14} />
            <span>Remote</span>
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            title={t.term_ai_helper_tooltip}
            onClick={(e) => {
               e.stopPropagation();
               window.dispatchEvent(new CustomEvent('aiterm:ask-ai', { detail: {} }));
            }}
            style={{ display: "flex", alignItems: "center", gap: "6px" }}
          >
            <SparklesIcon size={14} />
            <span>Ask AI</span>
          </button>
        </span>
      </div>

      {/* Sub-tabs: Terminal | Files | CWD path */}
      <div className="aiterm-subtabs">
        <button
          className={`aiterm-subtab ${viewTab === "terminal" ? "aiterm-subtab--active" : ""}`}
          onClick={() => setViewTab("terminal")}
        >{t.terminal_tab}</button>
        <button
          className={`aiterm-subtab ${viewTab === "files" ? "aiterm-subtab--active" : ""}`}
          onClick={() => setViewTab("files")}
        >{t.files_tab}</button>
        {displayCwd && (
          <span className="aiterm-status-cwd" title={displayCwd}>
            <span className="aiterm-status-cwd-icon">📁</span>
            {displayCwd}
          </span>
        )}
      </div>


      <div style={{ position: "relative", flex: 1, minHeight: 0, width: "100%", display: "flex", flexDirection: "column" }}>
        {/* File Explorer */}
        {viewTab === "files" && sessionId && (
          <div style={{ height: "100%", overflow: "hidden" }}>
            <FileExplorer
              sessionId={sessionId}
              onSwitchTerminalHere={(path) => {
                submitCommand(`cd "${path}"`);
                setViewTab("terminal");
              }}
            />
          </div>
        )}
        {/* Terminal */}
        <div
          ref={blockListRef}
          // Identifies the container the live pane's locked wheel forwards to
          // — see onLiveWheel.
          data-aiterm-live-scroll-target="1"
          style={{
            display: viewTab === "terminal" ? "flex" : "none",
            flexDirection: "column",
            height: "100%",
            position: "relative",
            overflowY: "auto",
            // 水平方向一律不可捲動。CSS 規則是「一軸非 visible 時，另一軸的
            // visible 會被計算成 auto」，所以只寫 overflowY:auto 會讓這個容器
            // 意外變成可水平捲動的 —— 瀏覽器就會拿它來「捲去顯示取得焦點的
            // 元素」，也就是 xterm 那個隱藏的 helper textarea。
            //
            // 症狀：Claude Code 這類把游標停在遠右欄位的 TUI，會讓整個畫面
            // 左移一個字元（`aiterm:opus` 顯示成 `iterm:opus`、`manual mode`
            // 顯示成 `anual mode`），而注音組字框跟著 textarea 跑到右下角。
            // vim 不會，因為它把游標留在插入點。
            //
            // 用 clip 不是 hidden：hidden 仍然是捲動容器（程式與瀏覽器都還能
            // 捲它），clip 才真的不是。xterm 自己處理內容的換行與捲動，這一層
            // 的水平捲動永遠是假的。
            overflowX: "clip",
            // Reserves the scrollbar's width at all times, whether or not it's
            // actually showing. Without this, the live pane below (a sibling
            // of the block-card list, both children of this scroll container)
            // width-percentage its way to a few pixels narrower the instant a
            // new card makes the block list tall enough to need a scrollbar —
            // which happens to land right when a command finishes. FitAddon's
            // ResizeObserver picks that up and calls term.resize(), firing a
            // real PTY resize; on Windows, ConPTY responds by re-transmitting
            // its currently-visible screen content, overwriting the
            // freshly-cleared prompt with stale output. Root-caused via
            // DevTools: logged every resize source and matched the timing and
            // magnitude (~12px, a scrollbar's width) exactly.
            scrollbarGutter: "stable",
          }}
          onMouseDown={(e) => {
            // Clicking the empty terminal area returns keyboard focus to the
            // live terminal so the user can type at the prompt, matching
            // standard terminal behavior. Skip clicks on cards, controls, and
            // the xterm host itself (xterm handles its own focus) so we don't
            // hijack text-selection or button clicks.
            const target = e.target as HTMLElement;
            if (target.closest('button, a, input, textarea, [id^="aiterm-block-"], .aiterm-terminal-root')) return;
            // preventDefault is essential: a mousedown on a non-focusable
            // element otherwise blurs the active element as its default action,
            // which runs AFTER this handler — so without it the browser would
            // immediately undo the focus() below and the caret would never
            // return to the prompt (the reported bug).
            e.preventDefault();
            termRef.current?.focus();
          }}
        >
        {/* Find in Buffer search bar */}
        {searchOpen && (
          <div className="terminal-search-bar">
            <input
              ref={searchInputRef}
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') { e.preventDefault(); closeSearch(); }
                if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); doSearch(searchQuery, 'next'); }
                if (e.key === 'Enter' && e.shiftKey) { e.preventDefault(); doSearch(searchQuery, 'prev'); }
                if (e.key === 'F3' && !e.shiftKey) { e.preventDefault(); doSearch(searchQuery, 'next'); }
                if (e.key === 'F3' && e.shiftKey) { e.preventDefault(); doSearch(searchQuery, 'prev'); }
              }}
              placeholder={t.term_search_placeholder}
              className="terminal-search-input"
            />
            {searchMatchInfo && <span className="terminal-search-match-info">{searchMatchInfo}</span>}
            <button onClick={() => doSearch(searchQuery, 'prev')} title={t.term_search_prev} className="terminal-search-btn aiterm-btn aiterm-btn--secondary aiterm-btn--sm">↑</button>
            <button onClick={() => doSearch(searchQuery, 'next')} title={t.term_search_next} className="terminal-search-btn aiterm-btn aiterm-btn--secondary aiterm-btn--sm">↓</button>
            <button onClick={closeSearch} title={t.term_search_close} className="terminal-search-btn terminal-search-close aiterm-btn aiterm-btn--secondary aiterm-btn--sm">✕</button>
          </div>
        )}
        {/* Block list is hidden while a full-screen program (vim, htop, less, ...) owns the
            alternate buffer — those programs must render exactly as they did before this
            refactor: full panel, no stale completed-command cards competing for space. */}
        {!isAlternateBuffer && (
          <div className="aiterm-block-list">
            {blocks
              .filter((b) => b.status !== "running" && b.renderedLines)
              .map((b) => (
                <div id={`aiterm-block-${b.id}`} key={b.id}>
                  <TerminalBlockCard
                    block={b}
                    highlightQuery={searchOpen && blockSearchCursor?.blockId === b.id ? searchQuery : undefined}
                    onAskAi={(command, exitCode) => {
                      window.dispatchEvent(new CustomEvent("aiterm:ask-ai", { detail: { command, exitCode } }));
                    }}
                    onBookmark={(command) => addBookmark(command)}
                    onCopy={(command) => navigator.clipboard.writeText(command).catch(console.error)}
                  />
                </div>
              ))}
          </div>
        )}
        {/* Outer wrapper clips + frames the live view; hostRef itself always stays a
            fixed 220px internally so its ResizeObserver never sees a size change from
            this — only this wrapper's height/overflow changes, so xterm's actual row
            count (and the PTY size it reports to the shell) is never affected. */}
        <div
          className="aiterm-live-frame"
          style={{
            // `100%` alone overflows the parent by exactly the frame's own
            // vertical margin (6px top + 6px bottom = 12px), since `height`
            // sizes only the box, not the margin around it — confirmed via
            // diagnostic logging: container scrollHeight was consistently
            // 12px taller than its clientHeight the instant a full-screen
            // TUI (e.g. Claude Code) filled this frame. Subtracting it here
            // makes box + margin add up to exactly 100%, so the outer
            // `overflow-y: auto` container never needs to scroll.
            height: isAlternateBuffer ? "calc(100% - 12px)" : `${liveHeightPx}px`,
            width: "calc(100% - 16px)",
            margin: "6px 8px",
            boxSizing: "border-box",
            flexShrink: 0,
            // `clip`, not `hidden`: both visually clip the fixed-220px xterm
            // host down to the live pane's height, but `clip` establishes NO
            // scroll container, so the browser physically cannot scroll this
            // element. `hidden` still allows programmatic/focus scrolling —
            // and the browser DID scroll it (to reveal xterm's focused
            // helper-textarea after a paste, and again on window resize),
            // pushing row 0 (the live prompt + typed/pasted command) up out of
            // the clipped viewport and leaving the pane looking blank/frozen
            // even though the buffer was correct. `clip` makes scrollIntoView &
            // friends skip this box entirely (they scroll the real container,
            // the block list, instead), fixing it at the source rather than
            // snapping scrollTop back after the fact. Supported in WebView2
            // (Chromium) and macOS WKWebView. The scroll-pin listener on mount
            // is a redundant safety net for the same failure mode.
            overflow: isAlternateBuffer ? "visible" : "clip",
            // Positioning context for the prompt-aligned host below.
            position: liveTopOffsetPx > 0 ? "relative" : undefined,
          }}
        >
          <div
            ref={hostRef}
            className={parkIme ? "aiterm-terminal-root aiterm-terminal-root--ime-park" : "aiterm-terminal-root"}
            style={{
              height: isAlternateBuffer ? "100%" : "220px",
              width: "100%",
              boxSizing: "border-box",
              // Windows keeps the xterm buffer in lockstep with ConPTY
              // (useTerminalBlocks no longer clears it — see the OSC 133 D
              // branch there for why), so the whole ConPTY screen is present
              // and "start at row 0" would show the oldest rows rather than
              // the prompt. Sliding the host up by the prompt's own
              // viewport-relative row makes the prompt the first visible row
              // wherever it currently sits — row 1 on a fresh tab, the last
              // row once the screen has filled. Other platforms clear the
              // buffer, so their prompt is already at row 0 and this is 0.
              ...(liveTopOffsetPx > 0
                ? { position: "absolute" as const, top: -liveTopOffsetPx, left: 0, right: 0, width: "auto" }
                : null),
            }}
          />
        </div>
        </div>{/* end terminal wrapper */}
      </div>{/* end relative container */}
      {/* WarpInput (the actual typing box) stays pinned to the panel bottom regardless of
          block-list length — only the live xterm view above scrolls with block content. */}
      {!isAlternateBuffer && agentPhase && (
        <AgentStatusBar
          status={agentPhase}
          onDismiss={() => setAgentPhase(null)}
          missionTokens={agentMission?.tokensUsed ?? 0}
        />
      )}
      {!isAlternateBuffer && (
        preview.loading && !agentPhase ? (
          <StreamingIndicator visible text={streamText} />
        ) : (
        <WarpInput
          sessionId={sessionId}
          onSubmit={(cmd) => {
            setAgentPhase(null);
            const agentQuery = parseAgentPrefix(cmd);
            const aiQuery = parseAiPrefix(cmd);
            if (agentQuery !== null || aiQuery !== null) {
              const finalQuery = agentQuery || aiQuery!;
              if (previewRef.current.loading) {
                termRef.current?.write("\r\n\x1b[31maiterm: already waiting for AI response\x1b[0m\r\n");
                return;
              }
              if (termRef.current && sessionId) {
                // Start Agent Loop (same as terminal Enter handler)
                agentAbortRef.current = false;
                startMission(finalQuery, 5);
                runAgentLoop({
                  t,
                  goal: finalQuery,
                  queryFn: (q) => invokeAiQuery(q, sessionId, locale),
                  term: termRef.current,
                  getSubmitCommand: () => submitCommandRef.current,
                  setPreview,
                  setStreamText,
                  streamingRef,
                  executionModeRef,
                  writeRed: (msg) => termRef.current?.write(`\r\n\x1b[31m${msg}\x1b[0m\r\n`),
                  abortRef: agentAbortRef,
                  stepCount: 0,
                  maxSteps: maxAgentStepsRef.current,
                  history: [],
                  onPhase: handleAgentPhase,
                  onComplete: () => {
                    setAgentPhase({ phase: "done", steps: agentStepRef.current });
                    stopMission();
                    if (sessionId) writePty(sessionId, "\r").catch(console.error);
                    onMissionEnd?.();
                  },
                  onFail: (msg) => {
                    setAgentPhase({ phase: "failed", reason: msg });
                    stopMission();
                    if (sessionId) writePty(sessionId, "\r").catch(console.error);
                    onMissionEnd?.();
                  },
                  onStepComplete: (info: AgentStepInfo) => reportAgentStep(info, { sendRemoteResponse, onAgentProgress }),
                });
              }
              return;
            }
            submitCommand(cmd);
          }}
          shortcut={submitShortcut}
        />
        )
      )}
      {preview.visible && (
        <CommandPreview
          command={preview.command}
          explanation={preview.explanation}
          riskLevel={preview.riskLevel}
          onConfirm={handleConfirm}
          onCancel={handleCancel}
        />
      )}
      {bookmarksOpen && (
        <CommandBookmarksPicker
          onSelect={(cmd) => {
            setBookmarksOpen(false);
            window.dispatchEvent(new CustomEvent("warp-fill-command", { detail: { cmd } }));
          }}
          onClose={() => setBookmarksOpen(false)}
        />
      )}
      {paletteOpen && (
        <ProviderPalette
          onClose={() => setPaletteOpen(false)}
          onSwitch={(p) => { setActiveProvider(p.display_name); setActiveProviderId(p.id); }}
        />
      )}
      {sessionId && (
        <AiPanel
          key={sessionId}
          sessionId={sessionId}
          isOpen={panelOpen}
          providerName={activeProvider}
          providerId={activeProviderId}
          onClose={() => setPanelOpen(false)}
          onExecuteCommand={(cmd, onComplete) => submitCommand(cmd, onComplete)}
          onOpenProviderPalette={() => {
            setPanelOpen(false);
            setPaletteOpen(true);
          }}
          sendRemoteResponse={sendRemoteResponse}
          getIdleMs={() => Date.now() - lastPtyOutputAtRef.current}
          onInterruptCommand={() => {
            // Ctrl+C：把 shell 從 heredoc／等輸入的狀態拉回提示字元。
            if (sessionRef.current) writePty(sessionRef.current, "\x03").catch(console.error);
            // 強制結案。finalizeBlock 會呼叫 agent 正在等的完成 callback
            // （useTerminalBlocks.ts:131-136），agent 迴圈因此自然接手下去，
            // 不需要另外通知它。
            const latest = blocksRef.current[blocksRef.current.length - 1];
            if (latest?.status === "running") finalizeBlock(latest.id, -1);
          }}
        />
      )}
    </div>
  );
}
