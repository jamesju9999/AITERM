import { useEffect, useState, useCallback, useRef } from "react";
import { TerminalView } from "./TerminalView";
import { TabBar, type Tab } from "./TabBar";
import { DatabaseView } from "./DatabaseView";
import { DesignView } from "./DesignView/DesignView";
import { CrossDbView } from "./CrossDbView";
import { useLocale } from "../contexts/LocaleContext";

const DEFAULT_TAB_STORAGE_KEY = "aiterm_default_tab";

export function TerminalApp() {
  const { t } = useLocale();
  const [tabs, setTabs] = useState<Tab[]>(() => {
    // Read from localStorage (sync) so the correct tab type shows immediately
    // without waiting for an async Rust config call.
    const saved = localStorage.getItem(DEFAULT_TAB_STORAGE_KEY);
    const tabType: "terminal" | "database" = saved === "database" ? "database" : "terminal";
    return [{ id: crypto.randomUUID(), title: tabType === "database" ? "Database" : "Terminal", type: tabType }];
  });
  const [activeId, setActiveId] = useState(tabs[0].id);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [isSidebarOpen, setIsSidebarOpen] = useState(true);
  const [sidebarWidth, setSidebarWidth] = useState(260);
  const [isDragging, setIsDragging] = useState(false);

  // We use refs to avoid binding stale values in keyboard listeners
  const tabsRef = useRef(tabs);
  const activeIdRef = useRef(activeId);
  const isSidebarOpenRef = useRef(isSidebarOpen);
  useEffect(() => {
    tabsRef.current = tabs;
    activeIdRef.current = activeId;
    isSidebarOpenRef.current = isSidebarOpen;
  }, [tabs, activeId, isSidebarOpen]);

  const handleAddTab = useCallback(() => {
    setPickerOpen(true);
  }, []);

  const handlePickerSelect = useCallback((type: "terminal" | "database" | "design" | "cross-db") => {
    const newId = crypto.randomUUID();
    let title = "Terminal";
    if (type === "database") title = t.database_tab;
    if (type === "design") title = "Design";
    if (type === "cross-db") title = t.cross_db_tab;
    setTabs((prev) => [...prev, { id: newId, title, type }]);
    setActiveId(newId);
    setPickerOpen(false);
  }, [t.database_tab, t.cross_db_tab]);

  const handleCloseTab = useCallback((id: string) => {
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

  // Sidebar drag to resize
  const startResizing = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsDragging(true);
  }, []);

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
    <div style={{ display: "flex", flexDirection: "row", height: "100vh", backgroundColor: "#0c0c0c" }}>
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
          pickerOpen={pickerOpen}
          onPickerSelect={handlePickerSelect}
          onPickerClose={() => setPickerOpen(false)}
        />
      </div>
      {isSidebarOpen && (
        <div
          style={{
            width: "6px",
            height: "100%",
            cursor: "col-resize",
            backgroundColor: isDragging ? "#4caf50" : "transparent",
            zIndex: 50,
            transition: "background-color 0.2s"
          }}
          onMouseDown={startResizing}
        />
      )}
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
              ) : (
                <TerminalView isActive={isActive} />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
