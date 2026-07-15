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
  RefreshIcon
} from "../Icons";
import "./index.css";

export type TabType = "terminal" | "database" | "design" | "cross-db" | "vcs" | "doc-converter" | "api-docs" | "loop-studio";

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
  hasUpdate = false
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

  if (!isSidebarOpen) {
    return (
      <div className="aiterm-tabbar aiterm-tabbar--collapsed" data-tauri-drag-region style={{ width: '48px', padding: '16px 0', alignItems: 'center', justifyContent: "space-between" }}>
        <button className="aiterm-sidebar-toggle" onClick={onToggle} title={t.tabbar_sidebar_expand} style={{ background: 'transparent', border: 'none', color: '#888', cursor: 'pointer', fontSize: '18px' }}>
          ◨
        </button>

        <button
          className="aiterm-sidebar-toggle"
          onClick={() => navigate("/settings", hasUpdate ? { state: { tab: "about" } } : undefined)}
          title={`${t.settings} (Ctrl+,)`}
          style={{ background: 'transparent', border: 'none', color: '#888', cursor: 'pointer', fontSize: '18px', position: 'relative' }}
        >
          ⚙
          {hasUpdate && <span className="update-badge" aria-label="Update available" />}
        </button>
      </div>
    );
  }

  return (
    <div className="aiterm-tabbar" data-tauri-drag-region style={{ width: `${width}px` }}>
      {/* Top spacing and Logo */}
      <div className="aiterm-sidebar-header-wrapper" style={{ marginTop: '24px', display: 'flex', flexDirection: 'column', alignItems: 'center', gap: '8px' }}>
        <div className="aiterm-sidebar-logo">
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
        <button className="aiterm-sidebar-toggle" onClick={onToggle} title="Close Sidebar (Ctrl+B)" style={{ background: 'transparent', border: 'none', color: 'var(--text-muted)', cursor: 'pointer', fontSize: '14px', padding: 0 }}>
          ◧ Collapse
        </button>
      </div>

      {/* Tabs list */}
      <div className="aiterm-tabbar-tabs" data-tauri-drag-region>
        {tabs.map((tab, idx) => (
          <div
            key={tab.id}
            className={`aiterm-tab ${tab.id === activeId ? "active" : ""}`}
            onClick={() => onSelect(tab.id)}
            onDoubleClick={() => setEditingId(tab.id)}
            title={`Switch to Tab (Ctrl+${idx + 1}) — Double click to rename`}
          >
            <span className="aiterm-tab-icon">{getTabIcon(tab.type)}</span>
            
            {editingId === tab.id ? (
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
            )}
            
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
          </div>
        ))}
        
        {/* Add Tab Button */}
        <button className="aiterm-tab-add-block" onClick={onAdd} title="New Tab (Ctrl+T)">
          <span style={{ fontSize: '18px' }}>+</span>
          <span style={{ fontSize: '10px', textTransform: 'uppercase', fontWeight: 600 }}>Add Tab</span>
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
          <span className="aiterm-tab-title">{t.settings}</span>
        </div>
      </div>
    </div>
  );
}
