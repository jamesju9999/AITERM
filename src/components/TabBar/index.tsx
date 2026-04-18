import { useState, useRef, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { NewTabPicker } from "../NewTabPicker";
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

export interface Tab {
  id: string;
  title: string;
  type: "terminal" | "database";
  dbConnectionId?: string;
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
  pickerOpen?: boolean;
  onPickerSelect?: (type: "terminal" | "database") => void;
  onPickerClose?: () => void;
}

export function TabBar({ tabs, activeId, onSelect, onClose, onAdd, onRename, isSidebarOpen, onToggle, width, pickerOpen, onPickerSelect, onPickerClose }: TabBarProps) {
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
      <div className="aiterm-tabbar aiterm-tabbar--collapsed" data-tauri-drag-region style={{ width: '48px', padding: '12px 0', alignItems: 'center', justifyContent: "space-between" }}>
        <button className="aiterm-sidebar-toggle" onClick={onToggle} title="Open Sidebar (Ctrl+B)" style={{ background: 'transparent', border: 'none', color: '#888', cursor: 'pointer', fontSize: '18px' }}>
          ◨
        </button>

        <button 
          className="aiterm-sidebar-toggle" 
          onClick={() => navigate("/settings")} 
          title={`${t.settings} (Ctrl+,)`}
          style={{ background: 'transparent', border: 'none', color: '#888', cursor: 'pointer', fontSize: '18px' }}
        >
          ⚙
        </button>
      </div>
    );
  }

  return (
    <div className="aiterm-tabbar" data-tauri-drag-region style={{ width: `${width}px` }}>
      <div className="aiterm-tabbar-header" data-tauri-drag-region style={{ position: "relative" }}>
        <button className="aiterm-sidebar-toggle" onClick={onToggle} title="Close Sidebar (Ctrl+B)" style={{ background: 'transparent', border: 'none', color: '#888', cursor: 'pointer', fontSize: '18px', padding: 0 }}>
          ◧
        </button>
        <span style={{ fontSize: "11px", fontWeight: "bold", color: "#888", letterSpacing: "1px", marginLeft: "4px" }}>AITerm Tabs</span>
        <button className="aiterm-tab-add" onClick={onAdd} title="New Tab (Ctrl+T)">
          +
        </button>
        {pickerOpen && onPickerSelect && onPickerClose && (
          <NewTabPicker onSelect={onPickerSelect} onClose={onPickerClose} />
        )}
      </div>
      <div className="aiterm-tabbar-tabs" data-tauri-drag-region>
        {tabs.map((tab, idx) => (
          <div
            key={tab.id}
            className={`aiterm-tab ${tab.id === activeId ? "active" : ""}`}
            onClick={() => onSelect(tab.id)}
            onDoubleClick={() => setEditingId(tab.id)}
            title={`Switch to Tab (Ctrl+${idx + 1}) — 雙擊重新命名`}
          >
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
      </div>

      <div className="aiterm-tabbar-footer" style={{ borderTop: "1px solid #2a2a2a", paddingTop: "8px", marginTop: "auto" }}>
        <div
            className="aiterm-tab"
            style={{ padding: "0 8px" }}
            onClick={() => navigate("/settings")}
            title={`${t.settings} (Ctrl+,)`}
        >
            <span style={{ marginRight: "8px", fontSize: "16px" }}>⚙</span>
            <span className="aiterm-tab-title">{t.settings}</span>
        </div>
      </div>
    </div>
  );
}
