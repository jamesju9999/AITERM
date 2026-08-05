import { useState, useRef, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { useLocale } from "../../contexts/LocaleContext";
import appIcon from "../../assets/icon.png";
import {
  TerminalIcon,
  DatabaseIcon,
  PaintbrushIcon,
  LinkIcon,
  LeafIcon,
  FileTextIcon,
  BookOpenIcon,
  RefreshIcon,
  PanelLeftCloseIcon,
  PanelLeftOpenIcon,
  CodeIcon,
  LibraryIcon,
  MailIcon
} from "../Icons";
import "./index.css";

export type TabType = "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio" | "code-assistant" | "knowledge-base" | "mail";

export interface Tab {
  id: string;
  title: string;
  type: TabType;
  dbConnectionId?: string;
  // Terminal-specific optional fields
  ptySessionId?: string;
  initialCwd?: string;
  initialMission?: { goal: string; maxSteps: number };
  enterpriseTask?: { taskId: string; workBranch: string; onComplete: unknown };
  agentProgress?: { done: number; total: number };
  /** AI-generated one-line summary of this tab's executed shell commands,
   *  shown in the title bar as "<title> - <aiSummary>". In-memory only —
   *  never persisted to localStorage, regenerated after the app restarts. */
  aiSummary?: string;
}

export interface TabBarProps {
  tabs: Tab[];
  activeId: string;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onAdd: () => void;
  onRename?: (id: string, title: string) => void;
  isSidebarOpen: boolean;
  onToggle: () => void;
  width: number;
  hasUpdate?: boolean;
  mailUnreadCount?: number;
}

function getTabIcon(type: TabType): React.ReactNode {
  switch (type) {
    case "terminal": return <TerminalIcon size={18} />;
    case "database": return <DatabaseIcon size={18} />;
    case "design": return <PaintbrushIcon size={18} />;
    case "cross-db": return <LinkIcon size={18} />;
    case "vcs": return <LeafIcon size={18} />;
    case "doc-converter": return <FileTextIcon size={18} />;
    case "api-docs": return <BookOpenIcon size={18} />;
    case "loop-studio": return <RefreshIcon size={18} />;
    case "code-assistant": return <CodeIcon size={18} />;
    case "knowledge-base": return <LibraryIcon size={18} />;
    case "mail": return <MailIcon size={18} />;
    default: return <FileTextIcon size={18} />;
  }
}

export function TabBar({
  tabs,
  activeId,
  onSelect,
  onClose,
  onAdd,
  onRename,
  isSidebarOpen,
  onToggle,
  width,
  hasUpdate = false,
  mailUnreadCount = 0
}: TabBarProps) {
  const navigate = useNavigate();
  const { t } = useLocale();
  const [editingId, setEditingId] = useState<string | null>(null);
  const editInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editingId && editInputRef.current) {
      editInputRef.current.focus();
      editInputRef.current.select();
    }
  }, [editingId]);

  return (
    <div
      className={`aiterm-tabbar ${!isSidebarOpen ? "aiterm-tabbar--collapsed" : ""}`}
      data-tauri-drag-region
      style={{ width: isSidebarOpen ? `${width}px` : "48px" }}
    >
      {/* Top spacing and Logo */}
      <div className="aiterm-sidebar-header-wrapper" style={{ marginTop: '24px', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: '8px', width: '100%' }}>
        {isSidebarOpen ? (
          <>
            <div className="aiterm-sidebar-logo" style={{ marginBottom: '4px' }}>
              <img
                src={appIcon}
                alt="AITerm"
                style={{
                  width: '38px',
                  height: '38px',
                  borderRadius: '12px',
                  boxShadow: '0 0 12px var(--accent-glow)',
                  display: 'block',
                }}
              />
            </div>
            <button
              className="aiterm-sidebar-toggle-block"
              onClick={onToggle}
              title="Close Sidebar (Ctrl+B)"
            >
              <span className="aiterm-tab-icon"><PanelLeftCloseIcon size={18} /></span>
              <span className="aiterm-tab-title">Collapse</span>
            </button>
          </>
        ) : (
          <>
            <div className="aiterm-sidebar-logo" style={{ marginBottom: '4px' }}>
              <img
                src={appIcon}
                alt="AITerm"
                style={{
                  width: '26px',
                  height: '26px',
                  borderRadius: '8px',
                  boxShadow: '0 0 8px var(--accent-glow)',
                  display: 'block',
                }}
              />
            </div>
            <button
              className="aiterm-sidebar-toggle-block"
              onClick={onToggle}
              title={t.tabbar_sidebar_expand}
            >
              <span className="aiterm-tab-icon"><PanelLeftOpenIcon size={18} /></span>
            </button>
          </>
        )}
      </div>

      {/* Tabs list */}
      <div className="aiterm-tabbar-tabs" data-tauri-drag-region>
        {tabs.map((tab, idx) => (
          <div
            key={tab.id}
            className={`aiterm-tab ${tab.id === activeId ? "active" : ""}`}
            onClick={() => onSelect(tab.id)}
            onDoubleClick={isSidebarOpen ? () => setEditingId(tab.id) : undefined}
            title={isSidebarOpen ? `Switch to Tab (Ctrl+${idx + 1}) — Double click to rename` : `${tab.title} (Ctrl+${idx + 1})`}
          >
            <span className="aiterm-tab-icon" style={{ position: "relative" }}>
              {getTabIcon(tab.type)}
              {tab.type === "mail" && mailUnreadCount > 0 && (
                <span className="mail-unread-badge" role="img" aria-label={`${mailUnreadCount > 99 ? "99+" : mailUnreadCount} ${t.mail_unread_label}`}>{mailUnreadCount > 99 ? "99+" : mailUnreadCount}</span>
              )}
            </span>

            {isSidebarOpen && (
              editingId === tab.id ? (
                <input
                  ref={editInputRef}
                  className="aiterm-tab-rename-input"
                  defaultValue={tab.title}
                  onBlur={(e) => {
                    const val = e.currentTarget.value.trim();
                    if (val && onRename) onRename(tab.id, val);
                    setEditingId(null);
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      const val = e.currentTarget.value.trim();
                      if (val && onRename) onRename(tab.id, val);
                      setEditingId(null);
                    } else if (e.key === "Escape") {
                      setEditingId(null);
                    }
                  }}
                  onClick={(e) => e.stopPropagation()}
                />
              ) : (
                <span className="aiterm-tab-title">{tab.title}</span>
              )
            )}
            
            {isSidebarOpen && (
              <button
                className="aiterm-tab-close"
                onClick={(e) => {
                  e.stopPropagation();
                  onClose(tab.id);
                }}
                title="Close Tab (Ctrl+W)"
              >
                ✕
              </button>
            )}
          </div>
        ))}
        
        {/* Add Tab Button */}
        <button className="aiterm-tab-add-block" onClick={onAdd} title="New Tab (Ctrl+T)">
          <span style={{ fontSize: '18px' }}>+</span>
          {isSidebarOpen && <span style={{ fontSize: '10px', textTransform: 'uppercase', fontWeight: 600 }}>Add Tab</span>}
        </button>
      </div>

      {/* Footer Area with Settings */}
      <div className="aiterm-tabbar-footer">
        <div
          className="aiterm-tab"
          onClick={() => navigate("/settings", hasUpdate ? { state: { tab: "about" } } : undefined)}
          title={`${t.settings} (Ctrl+,)`}
        >
          <span className="aiterm-tab-icon">⚙️</span>
          {isSidebarOpen && <span className="aiterm-tab-title">{t.settings}</span>}
          {hasUpdate && <span className={`update-badge ${isSidebarOpen ? "update-badge--tile" : ""}`} aria-label="Update available" />}
        </div>
      </div>
    </div>
  );
}
