import { useEffect, useState, useCallback, useRef } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { sendNotification } from "@tauri-apps/plugin-notification";
import { ensureNotificationPermission } from "../lib/notifyPermission";
import { routeAttention, notifyBodyKeyFor, isPastNotifyCooldown, type AttentionKind } from "../lib/terminalAttention";
import { TerminalView } from "./TerminalView";
import { TabBar, type Tab, type TabType } from "./TabBar";
import { reorderTabs } from "./TabBar/reorderTabs";
import { TitleBar } from "./TitleBar";
import { DatabaseView } from "./DatabaseView";
import { DesignView } from "./DesignView/DesignView";
import { NewTabPicker } from "./NewTabPicker";
import type { TabOpenOpts } from "./NewTabPicker/tabCatalog";
import { ConsentDialog } from "./ConsentDialog";
import { ConnectDialog } from "./ConnectDialog";
import { CrossDbView } from "./CrossDbView";
import { VcsView } from "./VcsView/VcsView";
import { DocConverterView } from "./DocConverter/DocConverterView";
import { ApiDocsView } from "./ApiDocsView";
import { LoopStudioView } from "./LoopStudio";
import { CodeAssistantView } from "./CodeAssistantView";
import { runCloseGuard } from "../lib/closeTabGuard";
import { KnowledgeBaseView } from "./KnowledgeBaseView";
import { MailView } from "./MailView";
import { RemoteTerminalView } from "./RemoteTerminalView";
import { HomeView } from "./HomeView";
import { RouteHint } from "./RouteHint";
import type { RouteResult } from "./HomeView/routeIntent";
import { useLocale } from "../contexts/LocaleContext";
import { setTabAgentProgress } from "../lib/tabAgentProgress";
import { restoreSessionTabs, saveSessionTabs } from "../lib/sessionTabs";
import { recordProject } from "../lib/recentProjects";
import { useMailSync } from "../hooks/useMailSync";
import { getConfig } from "../ipc/config";
import { bridgeStatus } from "../ipc/bridge";
import { onCoordinationTabSpawned } from "../ipc/mcpToolServer";
import {
  onEnterpriseTaskReceived,
  onEnterpriseTaskReady,
  onEnterpriseSkillInstalled,
  enterpriseAcceptTask,
  enterpriseRejectTask,
  type TaskPacket,
  type TaskReadyPayload,
  type SkillInstalledPayload,
} from "../ipc/enterprise";

const DEFAULT_TAB_STORAGE_KEY = "aiterm_default_tab";

interface TerminalAppProps {
  hasUpdate?: boolean;
  onClaudeDetected?: () => void;
}

export function TerminalApp({ hasUpdate = false, onClaudeDetected }: TerminalAppProps) {
  const { t } = useLocale();
  const [tabs, setTabs] = useState<Tab[]>(() => {
    // Try to restore previous session tabs first
    const restored = restoreSessionTabs();
    if (restored) return restored;
    // Fall back to default tab type from settings
    const saved = localStorage.getItem(DEFAULT_TAB_STORAGE_KEY);
    const tabType: "terminal" | "database" = saved === "database" ? "database" : "terminal";
    return [{ id: crypto.randomUUID(), title: tabType === "database" ? "Database" : "Terminal", type: tabType }];
  });
  const [activeId, setActiveId] = useState(tabs[0].id);
  // 首頁不是分頁，所以它不在 tabs 裡，而是一個「都不 active」的狀態。
  // 預設 true：開 app 先看到首頁，上次的分頁照常還原但不在前景。
  const [homeActive, setHomeActive] = useState(true);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [connectOpen, setConnectOpen] = useState(false);
  // 記住這次開啟 ConnectDialog，是不是某個既有遠端終端機分頁的工具列
  // 「連線」按鈕要求的——是的話，連線成功後要更新那個分頁，不是開新的。
  // null 代表這次是走 ADD TAB 的正常「開新分頁」流程。
  const [reconnectTabId, setReconnectTabId] = useState<string | null>(null);
  // 首頁 AI 路由猜對／猜錯的反悔提示：記住開出來的分頁 id、AI 選了什麼類型、
  // 使用者原句（換分頁類型時要用同一句重開）。null 代表沒有提示要顯示。
  const [routeHint, setRouteHint] = useState<{ tabId: string; type: TabType; userText: string } | null>(null);
  const [isSidebarOpen, setIsSidebarOpen] = useState(true);
  const [sidebarWidth, setSidebarWidth] = useState(76);
  const [isDragging, setIsDragging] = useState(false);
  // Mounted here, in the always-mounted shell, rather than in MailView:
  // important mail must notify the user even if the Mail tab was never opened.
  const {
    unreadCount: mailUnreadCount,
    failedAccountCount: mailFailedAccountCount,
    refreshUnread: refreshMailUnread,
  } = useMailSync();

  // We use refs to avoid binding stale values in keyboard listeners
  const tabsRef = useRef(tabs);
  const activeIdRef = useRef(activeId);
  const isSidebarOpenRef = useRef(isSidebarOpen);
  const homeActiveRef = useRef(homeActive);
  // Tab close guards: components register an async fn that returns true = ok to close, false = cancel
  const closeGuardsRef = useRef<Map<string, () => Promise<boolean>>>(new Map());
  const registerCloseGuard = useCallback((tabId: string, guard: () => Promise<boolean>) => {
    closeGuardsRef.current.set(tabId, guard);
  }, []);
  const unregisterCloseGuard = useCallback((tabId: string) => {
    closeGuardsRef.current.delete(tabId);
  }, []);
  // 視窗焦點放在 ref 而非 state：它只被事件 callback 讀取，不影響任何渲染，
  // 用 state 會讓每次切換視窗都重繪整個 app。初始值樂觀設為 true，
  // 這樣在 isFocused() 回來之前不會誤發通知。
  const windowFocusedRef = useRef(true);
  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    win.isFocused().then((f) => { windowFocusedRef.current = f; }).catch(() => {});
    win.onFocusChanged(({ payload }) => { windowFocusedRef.current = payload; })
      .then((u) => { unlisten = u; })
      .catch(() => {});
    return () => unlisten?.();
  }, []);

  // Claude Code 橋接：新建的一般終端機分頁要不要預設注入橋接環境變數。
  // 用 ref 而非 state——只有分頁建立當下的回呼會讀它，讀回來之後不需要重繪。
  //
  // 必須同時滿足「設定開著」與「server 真的在跑」。NewTabPicker 的 Claude Code
  // 選項本來就會在 server 沒啟動時停用（註解寫著「建立一個注入了死埠位址的分頁，
  // 比不給選更難除錯」），但預設值這條路徑原本只讀設定、完全繞過那個防護——
  // 於是選單擋著不讓建，一般終端機分頁卻照樣被注入指向死埠的環境變數。
  const defaultBridgeOnNewTabRef = useRef(false);
  const refreshDefaultBridge = useCallback(async () => {
    try {
      const [cfg, status] = await Promise.all([getConfig(), bridgeStatus()]);
      defaultBridgeOnNewTabRef.current = cfg.claude_bridge.default_on_new_tab && status.running;
    } catch {
      defaultBridgeOnNewTabRef.current = false;
    }
  }, []);
  useEffect(() => { void refreshDefaultBridge(); }, [refreshDefaultBridge]);

  // PTY session ID of the most recently active terminal tab — used by VcsView for CWD polling.
  const [lastTerminalPtyId, setLastTerminalPtyId] = useState<string>("");
  // 哪個終端機分頁目前是「唯一的 Telegram Remote 分頁」——天然互斥的單一真相
  // 來源。取代原本以「這個分頁是否可見」（isActive）當作監聽器閘門的作法：
  // 那個作法在首頁變成啟動預設畫面後，只要按首頁鍵就會主動 unlisten，
  // 期間收到的 Telegram 訊息永久遺失（Tauri 的 emit 找不到 listener 就直接
  // 丟棄，沒有 buffer）。null 代表沒有分頁開著 Remote。
  const [remoteTabId, setRemoteTabId] = useState<string | null>(null);
  useEffect(() => {
    tabsRef.current = tabs;
    activeIdRef.current = activeId;
    isSidebarOpenRef.current = isSidebarOpen;
    homeActiveRef.current = homeActive;
    // When switching to a terminal tab that already has a PTY, update the tracked ID.
    const activeTab = tabs.find((t) => t.id === activeId);
    if (activeTab?.type === "terminal" && activeTab.ptySessionId) {
      setLastTerminalPtyId(activeTab.ptySessionId);
    }
    // Persist tab layout for session restoration
    saveSessionTabs(tabs);
  }, [tabs, activeId, isSidebarOpen, homeActive]);

  // 切到某個分頁就把它的提示點清掉——使用者選定的規則是「切過去就算讀過」。
  // 這裡而不是用一個以 activeId 為依賴的 effect：清除在語意上是「選取分頁」
  // 的一部分，屬於事件本身，不是事後補償。也因此 active 分頁永遠不會有提示點。
  const selectTab = useCallback((id: string) => {
    setActiveId(id);
    setHomeActive(false);
    setTabs((prev) =>
      prev.some((t) => t.id === id && t.attention)
        ? prev.map((t) => (t.id === id ? { ...t, attention: undefined } : t))
        : prev
    );
  }, []);

  // Enterprise: pending task notification + skill toast
  const [pendingTask, setPendingTask] = useState<TaskPacket | null>(null);
  const [skillToast, setSkillToast] = useState<SkillInstalledPayload | null>(null);

  // Updated by each TerminalView's onRunningChange. Read (not written) by the
  // mcp-coordination-tab-spawned handler below to decide whether switching
  // focus to a newly agent-spawned tab would interrupt the user.
  const tabRunningRef = useRef<Map<string, boolean>>(new Map());

  // Listen for enterprise events
  useEffect(() => {
    let unlistenTaskReceived: (() => void) | null = null;
    let unlistenTaskReady: (() => void) | null = null;
    let unlistenSkill: (() => void) | null = null;
    let unlistenCoordination: (() => void) | null = null;
    let cancelled = false;

    onEnterpriseTaskReceived((packet) => {
      setPendingTask(packet);
    }).then((fn) => {
      if (cancelled) { fn(); return; }
      unlistenTaskReceived = fn;
    });

    onEnterpriseTaskReady((payload: TaskReadyPayload) => {
      // Create a new terminal tab that auto-starts the agent loop in the cloned repo.
      const newId = crypto.randomUUID();
      const goal = [
        payload.title,
        payload.description,
        payload.spec_content ? `\n\nSpec:\n${payload.spec_content}` : "",
      ].filter(Boolean).join("\n\n");

      setTabs((prev) => [
        ...prev,
        {
          id: newId,
          title: `⚙ ${payload.title.slice(0, 30)}`,
          type: "terminal" as const,
          initialCwd: payload.repo_dir,
          initialMission: { goal, maxSteps: payload.max_steps },
          enterpriseTask: {
            taskId: payload.task_id,
            workBranch: payload.work_branch,
            onComplete: payload.on_complete,
          },
        },
      ]);
      selectTab(newId);
    }).then((fn) => {
      if (cancelled) { fn(); return; }
      unlistenTaskReady = fn;
    });

    onEnterpriseSkillInstalled((payload) => {
      setSkillToast(payload);
      setTimeout(() => setSkillToast(null), 8000);
    }).then((fn) => {
      if (cancelled) { fn(); return; }
      unlistenSkill = fn;
    });

    onCoordinationTabSpawned((payload) => {
      const newId = crypto.randomUUID();
      const activeTab = tabsRef.current.find((tb) => tb.id === activeIdRef.current);
      const activeIsBusy = activeTab?.type === "terminal" && tabRunningRef.current.get(activeTab.id) === true;
      setTabs((prev) => [...prev, {
        id: newId,
        title: payload.command ? `Agent: ${payload.command}` : t.terminal_tab,
        type: "terminal",
        ptySessionId: payload.session_id,
        spawnedByAgent: true,
      }]);
      if (!activeIsBusy) {
        selectTab(newId);
      }
    }).then((fn) => {
      if (cancelled) { fn(); return; }
      unlistenCoordination = fn;
    });

    return () => {
      cancelled = true;
      unlistenTaskReceived?.();
      unlistenTaskReady?.();
      unlistenSkill?.();
      unlistenCoordination?.();
    };
  }, [selectTab, t.terminal_tab]);

  const handleAddTab = useCallback(() => {
    // 每次開選單都重讀：設定改過、或 server 中途啟動／停止，都不必重開 App。
    void refreshDefaultBridge();
    setPickerOpen(true);
  }, [refreshDefaultBridge]);

  const handlePickerSelect = useCallback((type: TabType, opts?: TabOpenOpts) => {
    if (type === "remote-terminal") {
      // 遠端分頁要先問短碼／位址才知道要連誰。連上之後才建分頁——
      // 那時才有 connId 可以掛事件。
      setConnectOpen(true);
      return "";
    }
    const newId = crypto.randomUUID();
    let title = t.terminal_tab;
    if (type === "database") title = t.database_tab;
    if (type === "design") title = t.design_tab;
    if (type === "cross-db") title = t.cross_db_tab;
    if (type === "vcs") title = t.vcs_tab;
    if (type === "doc-converter") title = t.doc_converter_tab;
    if (type === "api-docs") title = t.api_docs_tab;
    if (type === "loop-studio") title = t.loop_studio_tab;
    if (type === "code-assistant") title = t.code_assistant_tab;
    if (type === "knowledge-base") title = t.knowledge_base_tab;
    if (type === "mail") title = t.mail_tab;
    // 兩種來源要分得出來：選單挑「Claude Code」是使用者當下的意圖，換整顆圖示
    // 與標題才不會跟一般終端機混淆；設定的「新分頁預設啟用」則是背景行為，使用者
    // 點的是「終端機」，把它改名成 Claude Code 會讓人以為點錯了——那一種只在終端機
    // 圖示上加個徽章。其他分頁類型沒有 PTY，這個旗標對它們沒有意義。
    const claudeBridge: Tab["claudeBridge"] =
      type !== "terminal"
        ? undefined
        : opts?.claudeBridge
          ? "explicit"
          : defaultBridgeOnNewTabRef.current
            ? "default"
            : undefined;
    if (claudeBridge === "explicit") title = t.bridge_tab_title;
    setTabs((prev) => [...prev, {
      id: newId, title, type, claudeBridge,
      initialCwd: opts?.initialCwd,
      initialMission: opts?.initialMission,
    }]);
    selectTab(newId);
    setPickerOpen(false);
    // 回傳新分頁的 id：呼叫端（首頁的 AI 路由）需要知道自己開了哪一個，
    // 才能在猜錯時把它換掉。
    return newId;
  }, [t.terminal_tab, t.database_tab, t.design_tab, t.cross_db_tab, t.vcs_tab, t.doc_converter_tab, t.api_docs_tab, t.loop_studio_tab, t.code_assistant_tab, t.knowledge_base_tab, t.mail_tab, t.bridge_tab_title, selectTab]);

  // 首頁 AI 路由開出一個分頁（非降級結果）：記住它，讓 RouteHint 能在
  // 這個分頁上顯示「AI 判斷你要的是 X 分頁——不對？換成…」。
  const handleAiRouted = useCallback((tabId: string, route: RouteResult) => {
    setRouteHint({ tabId, type: route.type, userText: route.userText });
  }, []);

  const handleCloseTab = useCallback(async (id: string): Promise<boolean> => {
    const canClose = await runCloseGuard(
      id,
      activeIdRef.current,
      closeGuardsRef.current.get(id),
      setActiveId,
    );
    if (!canClose) return false;
    // 關掉的剛好是目前的 remote 分頁：釋放這個位置，不留著一個指向已經不存在
    // 分頁的 id。
    setRemoteTabId((prev) => (prev === id ? null : prev));
    // 同一個道理：ConnectDialog 開著、要求重新連線的正是這個分頁時，
    // Ctrl+W 把它關掉不能留著一個指向已經不存在分頁的 reconnectTabId——
    // 不然 ConnectDialog 完成連線流程時，會嘗試更新一個不存在的分頁。
    //
    // **不變量，下面 ConnectDialog.onConnected 依賴它**：這一行跟下面
    // 緊接著移除分頁的 setTabs 呼叫，兩者之間沒有 await 打斷，屬於同一次
    // React 批次更新——任何讀到其中一個新值的地方，一定也會同時讀到
    // 另一個。ConnectDialog.onConnected 因此可以放心假設「reconnectTabId
    // 讀到非 null，代表分頁必然還在」，不需要另外檢查一次（那邊的註解
    // 有寫這個推論，這裡是反向的提醒）。如果以後在這兩個 setState 中間
    // 插入 await，或是在別的地方新增第二個會移除分頁的路徑，這個不變量
    // 就會被打破，要一併檢查 onConnected 那邊的假設還成不成立。
    setReconnectTabId((prev) => (prev === id ? null : prev));
    // 這裡不能直接呼叫 selectTab：它內部也會呼叫 setTabs，巢狀呼叫等於在
    // 同一個 state 的更新佇列還在處理時再次 dispatch 同一個 state。改成
    // 清除跟著同一個 updater 的回傳值一起算，不另外呼叫 setTabs。
    setTabs((prev) => {
      const idx = prev.findIndex((t) => t.id === id);
      if (idx === -1) return prev;

      const nextTabs = prev.filter((t) => t.id !== id);
      // If we close the last tab, create a new fresh one so the window isn't empty
      if (nextTabs.length === 0) {
        const newId = crypto.randomUUID();
        setActiveId(newId);
        // 這條路徑是「關掉最後一個分頁時自動補一個」，不是使用者挑的，
        // 所以算 default 來源：標題維持字面的 Terminal，不改名。
        const bridged = defaultBridgeOnNewTabRef.current;
        return [{ id: newId, title: "Terminal", type: "terminal", claudeBridge: bridged ? "default" as const : undefined }];
      }

      // If closing active tab, switch to adjacent tab
      if (activeIdRef.current === id) {
        const nextActive = nextTabs[Math.min(idx, nextTabs.length - 1)];
        setActiveId(nextActive.id);
        // 切過去的鄰居可能正帶著提示點，一併清掉——與 selectTab 同一條規則，
        // 只是這裡在同一個 updater 裡純粹地做完，不需要第二次 setTabs。
        return nextTabs.map((t) => (t.id === nextActive.id ? { ...t, attention: undefined } : t));
      }
      return nextTabs;
    });
    return true;
  }, []);

  // RouteHint 的「換成…」：關掉猜錯的那個分頁，用同一句 userText 重開成
  // 使用者選的類型，並把提示狀態指向新分頁。
  const handleRouteHintPick = useCallback(async (type: TabType) => {
    if (!routeHint) return;
    const { tabId: oldId, userText } = routeHint;
    // 終端機／程式庫協助／LoopStudio 分頁都可能註冊 close guard，所以這裡
    // 一定要 await：否則確認框還開著，新分頁就先開出來搶走焦點，舊分頁會
    // 帶著一個看不見的對話框卡住關不掉。
    // 使用者取消關閉時也不該開新分頁——他要的是留在原地。
    const closed = await handleCloseTab(oldId);
    if (!closed) return;
    const opts: TabOpenOpts | undefined =
      type === "terminal" ? { initialMission: { goal: userText, maxSteps: 20 } } : undefined;
    const newId = handlePickerSelect(type, opts);
    setRouteHint({ tabId: newId, type, userText });
  }, [routeHint, handleCloseTab, handlePickerSelect]);

  const handleRename = useCallback((id: string, newTitle: string) => {
    setTabs((prev) => prev.map((t) => (t.id === id ? { ...t, title: newTitle } : t)));
  }, []);

  // 順序不需要另外儲存：saveSessionTabs 就是照陣列順序寫進 localStorage 的。
  const handleReorder = useCallback((from: number, to: number) => {
    setTabs((prev) => reorderTabs(prev, from, to));
  }, []);

  // 每個分頁各自的通知冷卻時間戳記。用 ref 而非 state：它只被
  // handleAttention 這個事件處理器讀寫，不影響任何渲染，用 state
  // 只會讓 handleAttention 每次都換一個新的參考。
  const notifyCooldownRef = useRef<Map<string, number>>(new Map());

  // 一個 attention 事件 → 兩個互相獨立的決定。規則本體在 routeAttention
  // （src/lib/terminalAttention.ts），那裡有單元測試釘住「提示點看分頁、
  // 通知看視窗焦點」這兩者不能被合併。
  const handleAttention = useCallback((tabId: string, tabTitle: string, kind: AttentionKind) => {
    const { badge, notify } = routeAttention({
      isActiveTab: activeIdRef.current === tabId,
      windowFocused: windowFocusedRef.current,
      kind,
    });

    if (badge) {
      setTabs((prev) => prev.map((t) => (t.id === tabId ? { ...t, attention: badge } : t)));
    }

    if (notify) {
      const bodyKey = notifyBodyKeyFor(kind);
      const now = Date.now();
      if (bodyKey && isPastNotifyCooldown(notifyCooldownRef.current.get(tabId), now)) {
        // 時間戳記要在這裡、同步地、在 ensureNotificationPermission() 這個
        // 非同步流程「開始之前」就寫入冷卻 Map——不是等它 resolve 之後才寫。
        // 這個分頁如果在權限 promise 回來之前又響了好幾次 bell，那些事件
        // 會在各自呼叫這裡時同步檢查冷卻 Map；如果時間戳記要等 resolve
        // 才寫入，這些事件會全部在還沒人動過 Map 的那一刻就通過冷卻檢查，
        // 排隊各自送出通知——冷卻機制形同虛設，防不了它原本要防的那種
        // 連環 bell。同步先佔位，才能讓同一個 tick 內的後續事件立刻被擋。
        notifyCooldownRef.current.set(tabId, now);
        ensureNotificationPermission().then((granted) => {
          if (granted) sendNotification({ title: tabTitle, body: t[bodyKey] });
        }).catch(() => { /* 通知失敗不是使用者能處理的事 */ });
      }
    }
  }, [t]);

  // Global Keyboard shortcuts for Tab Management
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // Don't intercept if modifier is missing
      if (!e.ctrlKey) return;
      
      if (e.key === "b" || e.key === "B") {
        e.preventDefault();
        setIsSidebarOpen((prev) => !prev);
      } else if (e.key === "t" || e.key === "T") {
        e.preventDefault();
        handleAddTab();
      } else if (e.key === "w" || e.key === "W") {
        e.preventDefault();
        // 首頁沒有可關的分頁——不擋掉會靜默關掉背景那個看不見的分頁。
        if (homeActiveRef.current) return;
        handleCloseTab(activeIdRef.current);
      } else if (e.key === "Tab") {
        e.preventDefault();
        const currentTabs = tabsRef.current;
        if (currentTabs.length <= 1) return;
        
        const currentId = activeIdRef.current;
        const idx = currentTabs.findIndex((t) => t.id === currentId);
        if (e.shiftKey) {
          // Prev tab
          const prevIdx = (idx - 1 + currentTabs.length) % currentTabs.length;
          selectTab(currentTabs[prevIdx].id);
        } else {
          // Next tab
          const nextIdx = (idx + 1) % currentTabs.length;
          selectTab(currentTabs[nextIdx].id);
        }
      } else if (e.key === "0") {
        // Windows/Linux 的 webview 用 Ctrl+0 重設縮放，一定要擋掉。
        // macOS 的重設縮放是 Cmd+0，不衝突。
        e.preventDefault();
        setHomeActive(true);
      } else if (e.key >= "1" && e.key <= "9") {
        // Go to specific tab (1-indexed)
        const i = parseInt(e.key, 10) - 1;
        const currentTabs = tabsRef.current;
        if (i >= 0 && i < currentTabs.length) {
          e.preventDefault();
          selectTab(currentTabs[i].id);
        }
      }
    };
    
    // Use capture phase to intercept before xterm.js potentially swallows them
    window.addEventListener("keydown", handleKeyDown, true);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, [handleAddTab, handleCloseTab, selectTab]);

  useEffect(() => {
    if (!isDragging) return;
    const onMouseMove = (e: MouseEvent) => {
      // Limit width between 180px and 600px
      const newWidth = Math.max(180, Math.min(e.clientX, 600));
      setSidebarWidth(newWidth);
    };
    const onMouseUp = () => {
      setIsDragging(false);
    };
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
    };
  }, [isDragging]);

  const toggleSidebar = useCallback(() => setIsSidebarOpen(o => !o), []);

  // 首頁不是分頁，標題列要退回 TitleBar 的預設值（"AITerm"），不能沿用
  // 背景分頁的標題／AI 摘要。
  const activeTabForTitle = homeActive ? undefined : tabs.find((t) => t.id === activeId);
  const titleBarText = activeTabForTitle
    ? (activeTabForTitle.type === "terminal" && activeTabForTitle.aiSummary
        ? `${activeTabForTitle.title} - ${activeTabForTitle.aiSummary}`
        : activeTabForTitle.title)
    : undefined;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", backgroundColor: "#0c0c0c", position: "relative" }}>
      <TitleBar title={titleBarText} />
      <div style={{ display: "flex", flexDirection: "row", flex: 1, minHeight: 0, position: "relative" }}>
      <div>
        <TabBar
          tabs={tabs}
          activeId={activeId}
          onSelect={selectTab}
          onClose={handleCloseTab}
          onAdd={handleAddTab}
          onRename={handleRename}
          onReorder={handleReorder}
          isSidebarOpen={isSidebarOpen}
          onToggle={toggleSidebar}
          width={sidebarWidth}
          hasUpdate={hasUpdate}
          mailUnreadCount={mailUnreadCount}
          mailFailedAccountCount={mailFailedAccountCount}
          onHome={() => setHomeActive(true)}
          homeActive={homeActive}
          remoteTabId={remoteTabId}
        />
      </div>
      
      {pickerOpen && (
        <div className="aiterm-new-tab-picker-popup" style={{ position: 'absolute', left: '80px', bottom: '60px', zIndex: 1000 }}>
          <NewTabPicker onSelect={handlePickerSelect} onClose={() => setPickerOpen(false)} />
        </div>
      )}
      <ConsentDialog tabs={tabs.map((x) => ({ id: x.id, title: x.title, ptySessionId: x.ptySessionId }))} />
      {connectOpen && (
        <ConnectDialog
          onCancel={() => {
            setConnectOpen(false);
            // 沒清的話，使用者從工具列按了連線鈕、又按取消，下一次改從
            // ADD TAB 開新分頁走正常流程，會被誤判成「這是剛才那個分頁
            // 要求的重新連線」，錯誤地更新舊分頁而不是開新分頁。
            setReconnectTabId(null);
          }}
          onConnected={(connId, sas, hostLabel) => {
            setConnectOpen(false);
            if (reconnectTabId) {
              const targetId = reconnectTabId;
              setReconnectTabId(null);
              // 這裡不需要另外判斷 targetId 指的分頁還在不在（先前兩版
              // 都在這裡加過一層「存不存在」檢查，一版用同步讀外部變數的
              // 寫法在正常路徑也誤判、一版可能永遠讀不到真正的競態）——
              // handleCloseTab（Ctrl+W 等關閉分頁的唯一入口，這個檔案裡
              // 移除分頁只有那一處 setTabs(prev => prev.filter(...))）
              // 在同一段沒有中斷的同步程式碼裡，把清空 reconnectTabId
              // 跟真正移除分頁這兩個 setState 一起呼叫——React 會把它們
              // 批次成同一次更新，任何讀到其中一個新值的地方，一定也會
              // 同時讀到另一個。也就是說：只要這裡的 reconnectTabId 讀到
              // 非 null，代表移除分頁那次更新根本還沒發生，targetId 指的
              // 分頁在這個當下必然還在——不可能出現「reconnectTabId 還
              // 沒清、但分頁已經不見」這種中間狀態，不需要再檢查一次。
              setTabs((prev) =>
                prev.map((tab) =>
                  tab.id === targetId
                    ? {
                        ...tab,
                        title: `${t.remote_terminal_tab}：${hostLabel}`,
                        remoteConnId: connId,
                        remoteHostLabel: hostLabel,
                        remoteSas: sas,
                      }
                    : tab,
                ),
              );
              selectTab(targetId);
              return;
            }
            const newId = crypto.randomUUID();
            setTabs((prev) => [
              ...prev,
              {
                id: newId,
                title: `${t.remote_terminal_tab}：${hostLabel}`,
                type: "remote-terminal",
                remoteConnId: connId,
                remoteHostLabel: hostLabel,
                // 這一端算出的驗證碼，要唸給對方核對。跟著連線的回傳值走而
                // 不是事件——事件會在這個分頁掛載之前就發出去。
                remoteSas: sas,
              },
            ]);
            selectTab(newId);
          }}
        />
      )}
      {/* Resizer divider disabled for layout [2] fixed 76px slim sidebar */}
      <div style={{ flex: 1, position: "relative", minWidth: 0 }}>
        {/* 首頁蓋在同一塊內容區。分頁一律留在 DOM 裡（見下方 isActive 附近的
            註解），所以這裡不能改成三元運算把分頁換掉。 */}
        {homeActive && (
          <HomeView onOpenTab={handlePickerSelect} tabs={tabs} onSelectTab={selectTab} onAiRouted={handleAiRouted} />
        )}
        {/* AI 路由猜錯分頁類型的反悔提示：只在猜出來的那個分頁正在前景時顯示。 */}
        {!homeActive && routeHint && routeHint.tabId === activeId && (
          <RouteHint
            pickedType={routeHint.type}
            onPick={handleRouteHintPick}
            onDismiss={() => setRouteHint(null)}
          />
        )}
        {tabs.map((tab) => {
          const isActive = tab.id === activeId && !homeActive;
          return (
            <div
              key={tab.id}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                right: 0,
                bottom: 0,
                // HIDDEN LAYOUT TRICK: 
                // Do NOT use display:none or unmount. Xterm.js will crash on resize if it has no dimensions!
                visibility: isActive ? "visible" : "hidden",
                zIndex: isActive ? 1 : -1,
                pointerEvents: isActive ? "auto" : "none",
              }}
            >
              {tab.type === "database" ? (
                <DatabaseView
                  tabId={tab.id}
                  isActive={isActive}
                  dbConnectionId={tab.dbConnectionId}
                  onConnectionSelected={(connId) => {
                    setTabs((prev) =>
                      prev.map((t) => t.id === tab.id ? { ...t, dbConnectionId: connId } : t)
                    );
                  }}
                  remoteOwner={remoteTabId}
                  onRemoteOwnerChange={setRemoteTabId}
                />
              ) : tab.type === "design" ? (
                <DesignView isActive={isActive} tabId={tab.id} remoteOwner={remoteTabId} onRemoteOwnerChange={setRemoteTabId} />
              ) : tab.type === "cross-db" ? (
                <CrossDbView tabId={tab.id} remoteOwner={remoteTabId} onRemoteOwnerChange={setRemoteTabId} />
              ) : tab.type === "vcs" ? (
                <VcsView sessionId={lastTerminalPtyId} isActive={isActive} />
              ) : tab.type === "doc-converter" ? (
                <DocConverterView isActive={isActive} />
              ) : tab.type === "api-docs" ? (
                <ApiDocsView isActive={isActive} />
              ) : tab.type === "loop-studio" ? (
                <LoopStudioView
                  sessionId={lastTerminalPtyId ?? undefined}
                  tabId={tab.id}
                  registerCloseGuard={registerCloseGuard}
                  unregisterCloseGuard={unregisterCloseGuard}
                />
              ) : tab.type === "code-assistant" ? (
                <CodeAssistantView
                  isActive={isActive}
                  tabId={tab.id}
                  registerCloseGuard={registerCloseGuard}
                  unregisterCloseGuard={unregisterCloseGuard}
                />
              ) : tab.type === "knowledge-base" ? (
                <KnowledgeBaseView isActive={isActive} />
              ) : tab.type === "mail" ? (
                <MailView isActive={isActive} onMessageRead={refreshMailUnread} />
              ) : tab.type === "remote-terminal" ? (
                // key={tab.remoteConnId}：連線切換時強制 React 把舊的
                // RemoteTerminalView 整個卸載、掛一個全新的實例，而不是
                // 保留舊實例只換 props。RemoteTerminalView 內部有十幾個
                // 只在掛載當下初始化一次的 state（phase、connectedAtRef/
                // elapsedMs、liveRows、hostRows、bookmarksOpen、
                // agentPanelOpen、hostPlatform，還有 useTerminalBlocks
                // 自己的 blocks/isAlternateBuffer，以及 xterm 實例本身）
                // ——connId prop 換了值不會讓它們自動歸零，用 key 換掉
                // 整個實例才能保證乾淨的起始狀態，不需要在元件內部逐一
                // 手動清空、也不會有漏清某個 state 的風險。舊實例卸載
                // 時，既有的斷線 effect（disconnectTimerRef，[connId]
                // 依賴）會照常觸發，正確斷掉舊連線，不需要另外處理。
                <RemoteTerminalView
                  key={tab.remoteConnId}
                  tabId={tab.id}
                  connId={tab.remoteConnId ?? ""}
                  sas={tab.remoteSas ?? ""}
                  isActive={isActive}
                  hostLabel={tab.remoteHostLabel ?? ""}
                  onConnectClick={() => {
                    setReconnectTabId(tab.id);
                    setConnectOpen(true);
                  }}
                />
              ) : (
                <TerminalView
                  isActive={isActive}
                  initialCwd={tab.initialCwd}
                  initialMission={tab.initialMission}
                  enterpriseTask={tab.enterpriseTask}
                  claudeBridge={tab.claudeBridge !== undefined}
                  onSessionCreated={(ptyId) => {
                    setTabs((prev) =>
                      prev.map((t) => t.id === tab.id ? { ...t, ptySessionId: ptyId } : t)
                    );
                    setLastTerminalPtyId(ptyId);
                  }}
                  onAgentProgress={(done, total) => {
                    setTabs((prev) => setTabAgentProgress(prev, tab.id, { done, total }));
                  }}
                  onMissionEnd={() => {
                    // 「進行中的任務」只該列真的在跑的——任務結束（成功或失敗）就清掉，
                    // 不用 status 欄位標記完成/失敗，那個訊號已經由 onAttention 負責。
                    setTabs((prev) => setTabAgentProgress(prev, tab.id, undefined));
                  }}
                  onSummaryUpdate={(summary) => {
                    setTabs((prev) =>
                      prev.map((t) => t.id === tab.id ? { ...t, aiSummary: summary } : t)
                    );
                  }}
                  onCwdChange={(cwd) => {
                    setTabs((prev) =>
                      prev.map((t) => t.id === tab.id ? { ...t, cwd } : t)
                    );
                    // 傳上一次的目錄進去：相同就不記錄。理由見 recordProject
                    // 的註解——沒有這個，開機本身就會洗掉整份最近專案清單。
                    recordProject(cwd, tab.cwd);
                  }}
                  onAttention={(kind) => handleAttention(tab.id, tab.title, kind)}
                  externalSessionId={tab.spawnedByAgent ? tab.ptySessionId : undefined}
                  onRunningChange={(isRunning) => { tabRunningRef.current.set(tab.id, isRunning); }}
                  onClaudeDetected={onClaudeDetected}
                  tabId={tab.id}
                  remoteOwner={remoteTabId}
                  onRemoteOwnerChange={setRemoteTabId}
                  registerCloseGuard={registerCloseGuard}
                  unregisterCloseGuard={unregisterCloseGuard}
                />
              )}
            </div>
          );
        })}
      </div>
      </div>{/* end sidebar+content row */}

      {/* Enterprise: Background Task Progress Panel (10.2) */}
      {(() => {
        // 首頁的 RunningTasks 已經涵蓋「顯示進行中任務」的職責且更完整
        // （不受 enterpriseTask / activeId 過濾限制），停在首頁時這個浮動
        // 面板不渲染，避免兩者重複出現、且較不完整的這個漏掉 activeId 那筆。
        if (homeActive) return null;
        const bgTasks = tabs.filter(
          (t) => t.type === "terminal" && t.enterpriseTask && t.agentProgress && t.id !== activeId
        );
        if (bgTasks.length === 0) return null;
        return (
          <div style={{
            position: "fixed", top: 16, right: 16, zIndex: 9998,
            display: "flex", flexDirection: "column", gap: 8,
          }}>
            {bgTasks.map((t) => {
              const prog = t.agentProgress!;
              const pct = Math.round((prog.done / Math.max(prog.total, 1)) * 100);
              return (
                <div
                  key={t.id}
                  onClick={() => selectTab(t.id)}
                  style={{
                    background: "#1e1e2e", border: "1px solid #3a3a6a", borderRadius: 6,
                    padding: "8px 14px", minWidth: 220, cursor: "pointer",
                    boxShadow: "0 2px 12px rgba(0,0,0,0.5)",
                  }}
                >
                  <div style={{ fontSize: 11, color: "#888", marginBottom: 4 }}>
                    ⚙ {t.title}
                  </div>
                  <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                    <div style={{ flex: 1, height: 4, background: "#333", borderRadius: 2 }}>
                      <div style={{ width: `${pct}%`, height: "100%", background: "#4a9eff", borderRadius: 2, transition: "width 0.3s" }} />
                    </div>
                    <span style={{ fontSize: 11, color: "#aaa", flexShrink: 0 }}>
                      {prog.done}/{prog.total}
                    </span>
                  </div>
                </div>
              );
            })}
          </div>
        );
      })()}

      {/* Enterprise: Task Notification Panel */}
      {pendingTask && (
        <div style={{
          position: "fixed", bottom: 24, right: 24, zIndex: 9999,
          background: "#1e1e2e", border: "1px solid #4a4a6a", borderRadius: 8,
          padding: "16px 20px", maxWidth: 360, color: "#e0e0f0",
          boxShadow: "0 4px 24px rgba(0,0,0,0.6)",
        }}>
          <div style={{ fontSize: 11, color: "#888", marginBottom: 4 }}>Enterprise Task</div>
          <div style={{ fontWeight: 600, marginBottom: 4 }}>{pendingTask.title}</div>
          <div style={{ fontSize: 13, color: "#aaa", marginBottom: 12, whiteSpace: "pre-wrap" }}>
            {pendingTask.description}
          </div>
          <div style={{ display: "flex", gap: 8 }}>
            <button
              onClick={() => {
                enterpriseAcceptTask(pendingTask).catch(console.error);
                setPendingTask(null);
              }}
              className="aiterm-btn aiterm-btn--primary"
              style={{ flex: 1 }}
            >
              Execute
            </button>
            <button
              onClick={() => {
                enterpriseRejectTask(pendingTask.task_id).catch(console.error);
                setPendingTask(null);
              }}
              className="aiterm-btn aiterm-btn--danger"
              style={{ flex: 1 }}
            >
              Reject
            </button>
          </div>
        </div>
      )}

      {/* Enterprise: Skill Installed Toast */}
      {skillToast && (
        <div style={{
          position: "fixed", bottom: 24, left: "50%", transform: "translateX(-50%)",
          zIndex: 9999, background: "#1e2e1e", border: "1px solid #4a6a4a", borderRadius: 8,
          padding: "12px 20px", color: "#e0f0e0",
          boxShadow: "0 4px 24px rgba(0,0,0,0.6)",
        }}>
          <span style={{ marginRight: 8 }}>✓</span>
          Skill installed: <strong>{skillToast.skill_id}</strong> v{skillToast.version}
          <button
            onClick={() => setSkillToast(null)}
            className="aiterm-btn aiterm-btn--ghost"
            style={{ marginLeft: 12 }}
          >
            ✕
          </button>
        </div>
      )}
    </div>
  );
}
