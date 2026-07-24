import { useEffect, useState, useCallback, useRef } from "react";
import { TerminalView } from "./TerminalView";
import { TabBar, type Tab } from "./TabBar";
import { TitleBar } from "./TitleBar";
import { DatabaseView } from "./DatabaseView";
import { DesignView } from "./DesignView/DesignView";
import { NewTabPicker } from "./NewTabPicker";
import { CrossDbView } from "./CrossDbView";
import { VcsView } from "./VcsView/VcsView";
import { DocConverterView } from "./DocConverter/DocConverterView";
import { ApiDocsView } from "./ApiDocsView";
import { LoopStudioView } from "./LoopStudio";
import { CodeAssistantView } from "./CodeAssistantView";
import { KnowledgeBaseView } from "./KnowledgeBaseView";
import { useLocale } from "../contexts/LocaleContext";
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
const SESSION_TABS_KEY = "aiterm-session-tabs";

type SavedTab = Pick<Tab, "title" | "type" | "dbConnectionId">;

function restoreSessionTabs(): Tab[] | null {
  try {
    const raw = localStorage.getItem(SESSION_TABS_KEY);
    if (!raw) return null;
    const saved: SavedTab[] = JSON.parse(raw);
    if (!Array.isArray(saved) || saved.length === 0) return null;
    return saved.map((s) => ({ ...s, id: crypto.randomUUID() }));
  } catch {
    return null;
  }
}

function saveSessionTabs(tabs: Tab[]) {
  const toSave: SavedTab[] = tabs.map(({ title, type, dbConnectionId }) => ({ title, type, dbConnectionId }));
  localStorage.setItem(SESSION_TABS_KEY, JSON.stringify(toSave));
}

interface TerminalAppProps {
  hasUpdate?: boolean;
}

export function TerminalApp({ hasUpdate = false }: TerminalAppProps) {
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
  const [pickerOpen, setPickerOpen] = useState(false);
  const [isSidebarOpen, setIsSidebarOpen] = useState(true);
  const [sidebarWidth, setSidebarWidth] = useState(76);
  const [isDragging, setIsDragging] = useState(false);

  // We use refs to avoid binding stale values in keyboard listeners
  const tabsRef = useRef(tabs);
  const activeIdRef = useRef(activeId);
  const isSidebarOpenRef = useRef(isSidebarOpen);
  // Tab close guards: components register an async fn that returns true = ok to close, false = cancel
  const closeGuardsRef = useRef<Map<string, () => Promise<boolean>>>(new Map());
  const registerCloseGuard = useCallback((tabId: string, guard: () => Promise<boolean>) => {
    closeGuardsRef.current.set(tabId, guard);
  }, []);
  const unregisterCloseGuard = useCallback((tabId: string) => {
    closeGuardsRef.current.delete(tabId);
  }, []);
  // PTY session ID of the most recently active terminal tab — used by VcsView for CWD polling.
  const [lastTerminalPtyId, setLastTerminalPtyId] = useState<string>("");
  useEffect(() => {
    tabsRef.current = tabs;
    activeIdRef.current = activeId;
    isSidebarOpenRef.current = isSidebarOpen;
    // When switching to a terminal tab that already has a PTY, update the tracked ID.
    const activeTab = tabs.find((t) => t.id === activeId);
    if (activeTab?.type === "terminal" && activeTab.ptySessionId) {
      setLastTerminalPtyId(activeTab.ptySessionId);
    }
    // Persist tab layout for session restoration
    saveSessionTabs(tabs);
  }, [tabs, activeId, isSidebarOpen]);

  // Enterprise: pending task notification + skill toast
  const [pendingTask, setPendingTask] = useState<TaskPacket | null>(null);
  const [skillToast, setSkillToast] = useState<SkillInstalledPayload | null>(null);

  // Listen for enterprise events
  useEffect(() => {
    let unlistenTaskReceived: (() => void) | null = null;
    let unlistenTaskReady: (() => void) | null = null;
    let unlistenSkill: (() => void) | null = null;

    onEnterpriseTaskReceived((packet) => {
      setPendingTask(packet);
    }).then((fn) => { unlistenTaskReceived = fn; });

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
      setActiveId(newId);
    }).then((fn) => { unlistenTaskReady = fn; });

    onEnterpriseSkillInstalled((payload) => {
      setSkillToast(payload);
      setTimeout(() => setSkillToast(null), 8000);
    }).then((fn) => { unlistenSkill = fn; });

    return () => {
      unlistenTaskReceived?.();
      unlistenTaskReady?.();
      unlistenSkill?.();
    };
  }, []);

  const handleAddTab = useCallback(() => {
    setPickerOpen(true);
  }, []);

  const handlePickerSelect = useCallback((type: "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio" | "code-assistant" | "knowledge-base") => {
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
    setTabs((prev) => [...prev, { id: newId, title, type }]);
    setActiveId(newId);
    setPickerOpen(false);
  }, [t.terminal_tab, t.database_tab, t.design_tab, t.cross_db_tab, t.vcs_tab, t.doc_converter_tab, t.api_docs_tab, t.loop_studio_tab, t.code_assistant_tab, t.knowledge_base_tab]);

  const handleCloseTab = useCallback(async (id: string) => {
    const guard = closeGuardsRef.current.get(id);
    if (guard) {
      const canClose = await guard();
      if (!canClose) return;
    }
    setTabs((prev) => {
      const idx = prev.findIndex((t) => t.id === id);
      if (idx === -1) return prev;
      
      const nextTabs = prev.filter((t) => t.id !== id);
      // If we close the last tab, create a new fresh one so the window isn't empty
      if (nextTabs.length === 0) {
        const newId = crypto.randomUUID();
        setActiveId(newId);
        return [{ id: newId, title: "Terminal", type: "terminal" }];
      }
      
      // If closing active tab, switch to adjacent tab
      if (activeIdRef.current === id) {
        const nextActive = nextTabs[Math.min(idx, nextTabs.length - 1)].id;
        setActiveId(nextActive);
      }
      return nextTabs;
    });
  }, []);

  const handleRename = useCallback((id: string, newTitle: string) => {
    setTabs((prev) => prev.map((t) => (t.id === id ? { ...t, title: newTitle } : t)));
  }, []);

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
          setActiveId(currentTabs[prevIdx].id);
        } else {
          // Next tab
          const nextIdx = (idx + 1) % currentTabs.length;
          setActiveId(currentTabs[nextIdx].id);
        }
      } else if (e.key >= "1" && e.key <= "9") {
        // Go to specific tab (1-indexed)
        const i = parseInt(e.key, 10) - 1;
        const currentTabs = tabsRef.current;
        if (i >= 0 && i < currentTabs.length) {
          e.preventDefault();
          setActiveId(currentTabs[i].id);
        }
      }
    };
    
    // Use capture phase to intercept before xterm.js potentially swallows them
    window.addEventListener("keydown", handleKeyDown, true);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, [handleAddTab, handleCloseTab]);

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

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh", backgroundColor: "#0c0c0c", position: "relative" }}>
      <TitleBar />
      <div style={{ display: "flex", flexDirection: "row", flex: 1, minHeight: 0, position: "relative" }}>
      <div>
        <TabBar
          tabs={tabs}
          activeId={activeId}
          onSelect={setActiveId}
          onClose={handleCloseTab}
          onAdd={handleAddTab}
          onRename={handleRename}
          isSidebarOpen={isSidebarOpen}
          onToggle={toggleSidebar}
          width={sidebarWidth}
          hasUpdate={hasUpdate}
        />
      </div>
      
      {pickerOpen && (
        <div className="aiterm-new-tab-picker-popup" style={{ position: 'absolute', left: '80px', bottom: '60px', zIndex: 1000 }}>
          <NewTabPicker onSelect={handlePickerSelect} onClose={() => setPickerOpen(false)} />
        </div>
      )}
      {/* Resizer divider disabled for layout [2] fixed 76px slim sidebar */}
      <div style={{ flex: 1, position: "relative", minWidth: 0 }}>
        {tabs.map((tab) => {
          const isActive = tab.id === activeId;
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
                />
              ) : tab.type === "design" ? (
                <DesignView isActive={isActive} />
              ) : tab.type === "cross-db" ? (
                <CrossDbView isActive={isActive} />
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
                <CodeAssistantView isActive={isActive} />
              ) : tab.type === "knowledge-base" ? (
                <KnowledgeBaseView isActive={isActive} />
              ) : (
                <TerminalView
                  isActive={isActive}
                  initialCwd={tab.initialCwd}
                  initialMission={tab.initialMission}
                  enterpriseTask={tab.enterpriseTask}
                  onSessionCreated={(ptyId) => {
                    setTabs((prev) =>
                      prev.map((t) => t.id === tab.id ? { ...t, ptySessionId: ptyId } : t)
                    );
                    setLastTerminalPtyId(ptyId);
                  }}
                  onAgentProgress={(done, total) => {
                    setTabs((prev) =>
                      prev.map((t) => t.id === tab.id ? { ...t, agentProgress: { done, total } } : t)
                    );
                  }}
                />
              )}
            </div>
          );
        })}
      </div>
      </div>{/* end sidebar+content row */}

      {/* Enterprise: Background Task Progress Panel (10.2) */}
      {(() => {
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
                  onClick={() => setActiveId(t.id)}
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
