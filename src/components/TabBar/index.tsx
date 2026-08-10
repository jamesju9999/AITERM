import { useState, useRef, useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { useLocale } from "../../contexts/LocaleContext";
import type { AttentionKind } from "../../lib/terminalAttention";
import type { Translations } from "../../lib/i18n";
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
  MailIcon,
  RobotIcon
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
  /** 非 active 的終端機分頁發生了值得注意的事：在側邊欄圖示上顯示一個彩色點。
   *  只存在記憶體，不進 localStorage——重開 app 後這些事件已經沒有意義。 */
  attention?: AttentionKind;
  /**
   * 這個終端機分頁有沒有注入 Claude Code 橋接環境變數，以及來源。
   *
   * 來源會影響外觀：`explicit`（使用者從選單挑了「Claude Code」）換整顆圖示與
   * 標題；`default`（設定的「新分頁預設啟用」）只在終端機圖示上加徽章——使用者
   * 點的是「終端機」，把它改名成 Claude Code 會讓人以為自己點錯了。
   */
  claudeBridge?: "explicit" | "default";
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
  /** Accounts whose mail server connection has broken. Anything above zero puts
   *  a warning marker on the Mail tab's icon — the only way a user who never
   *  opens the Mail tab learns their inbox has stopped updating. */
  mailFailedAccountCount?: number;
}

// Single source of truth for the cap rule, so the badge's visible text and its
// accessible name can never disagree.
function formatUnreadCount(n: number): string {
  return n > 99 ? "99+" : String(n);
}

function getTabIcon(type: TabType, claudeBridge?: Tab["claudeBridge"]): React.ReactNode {
  switch (type) {
    // 橋接分頁用機器人圖示——跟「新增分頁」選單裡 Claude Code 那一項同一顆，
    // 選單看到什麼、分頁就長什麼。原本是終端機圖示疊一顆 "CC" 徽章，兩者重疊，
    // 而且側邊欄收起來只剩圖示時，一疊小貼紙比換一顆圖示難認。
    case "terminal": return claudeBridge === "explicit" ? <RobotIcon size={18} /> : <TerminalIcon size={18} />;
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

// 顏色只對看得見的人有意義，所以每個狀態都要有自己的文字說明。
function attentionLabel(kind: AttentionKind, t: Translations): string {
  switch (kind) {
    case "waiting": return t.terminal_attention_waiting_label;
    case "done": return t.terminal_attention_done_label;
    case "failed": return t.terminal_attention_failed_label;
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
  mailUnreadCount = 0,
  mailFailedAccountCount = 0
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
              {getTabIcon(tab.type, tab.claudeBridge)}
              {tab.type === "mail" && mailUnreadCount > 0 && (
                <span className="mail-unread-badge" role="img" aria-label={t.mail_unread_label(formatUnreadCount(mailUnreadCount))}>{formatUnreadCount(mailUnreadCount)}</span>
              )}
              {/* Opposite corner from the unread pill so the two never overlap,
                  and deliberately not a count: "how many accounts are broken"
                  is not the point, "something is broken" is. */}
              {tab.type === "mail" && mailFailedAccountCount > 0 && (
                <span className="mail-connection-badge" role="img" aria-label={t.mail_connection_failed_label(String(mailFailedAccountCount))}>!</span>
              )}
              {/* 錨在圖示右下角。mail 的兩個 badge 只出現在 mail 分頁、
                  這個只出現在 terminal 分頁，位置不會互撞。 */}
              {tab.type === "terminal" && tab.attention && (
                <span
                  className={`terminal-attention-badge terminal-attention-badge--${tab.attention}`}
                  role="img"
                  aria-label={attentionLabel(tab.attention, t)}
                />
              )}
              {/* 只有 default 來源需要徽章——explicit 已經換了整顆圖示。錨在左上角，
                  跟右下角的 attention 點、mail 的兩個角落都不會撞。 */}
              {tab.type === "terminal" && tab.claudeBridge === "default" && (
                <span className="terminal-bridge-badge">{t.bridge_tab_badge}</span>
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
