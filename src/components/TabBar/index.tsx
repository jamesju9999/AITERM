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
  RobotIcon,
  HomeIcon
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
   *  shown in the title bar as "<title> - <aiSummary>". Persisted to
   *  localStorage so「接續上次的工作」可以顯示上次做到哪——重開 app 後
   *  不會再更新，直到這個分頁又執行新指令觸發下一次摘要。 */
  aiSummary?: string;
  /** 上一個 session 的 AI 摘要，由 restoreSessionTabs 還原。只給首頁的「接續
   *  上次的工作」讀——標題列不能用它，那裡沒有「上次」的框架。這個 session
   *  一旦跑出新摘要，aiSummary 就會蓋過它。 */
  lastSessionSummary?: string;
  /** 這個終端機分頁目前實際所在的工作目錄。由 TerminalView 回報，會持久化。
   *  跟 initialCwd（開分頁時的起始目錄）是兩件事，不要混用。 */
  cwd?: string;
  /** 非 active 的終端機分頁發生了值得注意的事：在側邊欄圖示上顯示一個彩色點。
   *  只存在記憶體，不進 localStorage——重開 app 後這些事件已經沒有意義。 */
  attention?: AttentionKind;
  /** 這個分頁是不是由 MCP 協調工具的 spawn_tab 開出來的——只影響圖示顯示，
   *  不影響任何行為。只存在記憶體，不進 localStorage。 */
  spawnedByAgent?: boolean;
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
  /** 使用者按了首頁。沒給就不顯示首頁按鈕。 */
  onHome?: () => void;
  /** 目前顯示的是不是首頁。 */
  homeActive?: boolean;
  /** 使用者把第 `from` 個分頁拖到第 `to` 個位置。沒給就不能拖曳。 */
  onReorder?: (from: number, to: number) => void;
  isSidebarOpen: boolean;
  onToggle: () => void;
  width: number;
  hasUpdate?: boolean;
  mailUnreadCount?: number;
  /** Accounts whose mail server connection has broken. Anything above zero puts
   *  a warning marker on the Mail tab's icon — the only way a user who never
   *  opens the Mail tab learns their inbox has stopped updating. */
  mailFailedAccountCount?: number;
  /** 目前唯一開著 Telegram Remote 的分頁 id（null 代表沒有）。修好首頁回歸之後
   *  背景分頁也會真的執行遠端指令，所以側邊欄需要一個訊號，不能只靠分頁
   *  內部那顆按鈕才看得到。 */
  remoteTabId?: string | null;
}

// Single source of truth for the cap rule, so the badge's visible text and its
// accessible name can never disagree.
function formatUnreadCount(n: number): string {
  return n > 99 ? "99+" : String(n);
}

function getTabIcon(type: TabType, claudeBridge?: Tab["claudeBridge"], spawnedByAgent?: boolean): React.ReactNode {
  switch (type) {
    // 橋接分頁用機器人圖示——跟「新增分頁」選單裡 Claude Code 那一項同一顆，
    // 選單看到什麼、分頁就長什麼。原本是終端機圖示疊一顆 "CC" 徽章，兩者重疊，
    // 而且側邊欄收起來只剩圖示時，一疊小貼紙比換一顆圖示難認。
    case "terminal":
      if (spawnedByAgent) return <RobotIcon size={18} className="tab-icon--agent-spawned" />;
      return claudeBridge === "explicit" ? <RobotIcon size={18} /> : <TerminalIcon size={18} />;
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

/** 小於這個位移量都當成單純點擊——不然點分頁切換會被誤判成拖曳。 */
const DRAG_THRESHOLD_PX = 4;

/** 拖曳期間的即時狀態。掛在 window 上的 mousemove/mouseup 會抓到建立當下的
 *  closure，所以這份資料只能放在 ref 裡，不能放 state。 */
interface DragState {
  from: number;
  to: number;
  startY: number;
  /** 拖曳開始當下、每個分頁的垂直中心。用來判斷游標落在哪一格。 */
  centers: number[];
  /** 一格的位移量（相鄰兩格中心的距離），讓位動畫用。 */
  row: number;
  /** 位移是否已經超過門檻。沒超過就還不算拖曳。 */
  started: boolean;
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
  onReorder,
  onHome,
  homeActive = false,
  isSidebarOpen,
  onToggle,
  width,
  hasUpdate = false,
  mailUnreadCount = 0,
  mailFailedAccountCount = 0,
  remoteTabId = null
}: TabBarProps) {
  const navigate = useNavigate();
  const { t } = useLocale();
  const [editingId, setEditingId] = useState<string | null>(null);
  const editInputRef = useRef<HTMLInputElement>(null);

  const tabRefs = useRef<(HTMLDivElement | null)[]>([]);
  const dragRef = useRef<DragState | null>(null);
  /** 拖曳後瀏覽器還會補一個 click，不擋掉的話拖完會順便切走分頁。 */
  const suppressClickRef = useRef(false);
  /** 只為了畫出讓位效果；判斷落點的真實來源是 dragRef。 */
  const [dragView, setDragView] = useState<{ from: number; to: number; row: number; dy: number } | null>(null);

  useEffect(() => {
    if (editingId && editInputRef.current) {
      editInputRef.current.focus();
      editInputRef.current.select();
    }
  }, [editingId]);

  // 拖曳一旦開始，游標可能離開分頁本身（甚至離開側邊欄），所以移動與放開都必須
  // 聽在 window 上，不能掛在分頁元素上。
  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      const st = dragRef.current;
      if (!st) return;
      const dy = e.clientY - st.startY;
      if (!st.started) {
        if (Math.abs(dy) < DRAG_THRESHOLD_PX) return;
        st.started = true;
      }
      // 從「被拖的分頁現在的中心」往兩邊掃，看越過了幾格的中心。
      const center = st.centers[st.from] + dy;
      let to = st.from;
      // 用 >= / <=：位移剛好整整一列時，就是剛好換一格。嚴格比較會讓「移動了
      // 一整格卻沒有換位」這種明顯錯誤的結果發生。
      if (dy > 0) {
        while (to < st.centers.length - 1 && center >= st.centers[to + 1]) to++;
      } else {
        while (to > 0 && center <= st.centers[to - 1]) to--;
      }
      st.to = to;
      setDragView({ from: st.from, to, row: st.row, dy });
    };
    const onUp = () => {
      const st = dragRef.current;
      dragRef.current = null;
      setDragView(null);
      if (!st?.started) return;
      suppressClickRef.current = true;
      if (st.to !== st.from) onReorder?.(st.from, st.to);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [onReorder]);

  const handleTabMouseDown = (e: React.MouseEvent, index: number) => {
    // 上一次拖曳如果沒收到收尾的 click（放開時游標已經不在原本那顆分頁上，
    // click 就不會發生），旗標會留著；在這裡清掉，才不會吃掉下一次點擊。
    suppressClickRef.current = false;
    if (!onReorder || e.button !== 0 || editingId === tabs[index].id) return;
    const rects = tabs.map((_, i) => tabRefs.current[i]?.getBoundingClientRect());
    if (rects.some((r) => !r)) return;
    const centers = rects.map((r) => r!.top + r!.height / 2);
    dragRef.current = {
      from: index,
      to: index,
      startY: e.clientY,
      centers,
      // 只有一個分頁時不會進到讓位邏輯，補 0 即可。
      row: centers.length > 1 ? centers[1] - centers[0] : 0,
      started: false,
    };
  };

  /** 拖曳中每個分頁該位移多少：被拖的那顆跟著游標，被越過的那幾顆讓出一格。 */
  const dragStyleOf = (i: number): React.CSSProperties | undefined => {
    if (!dragView) return undefined;
    const { from, to, row, dy } = dragView;
    if (i === from) return { transform: `translateY(${dy}px)` };
    if (from < to && i > from && i <= to) return { transform: `translateY(${-row}px)` };
    if (from > to && i >= to && i < from) return { transform: `translateY(${row}px)` };
    return undefined;
  };

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

      {/* 首頁不是分頁：它固定在這裡，不進 .aiterm-tabbar-tabs，所以不會被
          拖曳排序、也不佔 Ctrl+1~9 的編號。 */}
      {onHome && (
        <button
          className={`aiterm-tab aiterm-home-button ${homeActive ? "active" : ""}`}
          onClick={onHome}
          aria-current={homeActive ? "page" : undefined}
          title={`${t.home_tab} (Ctrl+0)`}
        >
          <span className="aiterm-tab-icon"><HomeIcon size={18} /></span>
          {isSidebarOpen && <span className="aiterm-tab-title">{t.home_tab}</span>}
        </button>
      )}

      {/* Tabs list */}
      <div className="aiterm-tabbar-tabs" data-tauri-drag-region>
        {tabs.map((tab, idx) => (
          <div
            key={tab.id}
            ref={(el) => { tabRefs.current[idx] = el; }}
            className={`aiterm-tab ${tab.id === activeId && !homeActive ? "active" : ""} ${dragView?.from === idx ? "aiterm-tab--dragging" : ""}`}
            style={dragStyleOf(idx)}
            onMouseDown={(e) => handleTabMouseDown(e, idx)}
            onClick={() => {
              if (suppressClickRef.current) {
                suppressClickRef.current = false;
                return;
              }
              onSelect(tab.id);
            }}
            onDoubleClick={isSidebarOpen ? () => setEditingId(tab.id) : undefined}
            title={
              (isSidebarOpen ? `Switch to Tab (Ctrl+${idx + 1}) — Double click to rename` : `${tab.title} (Ctrl+${idx + 1})`)
              + (tab.spawnedByAgent ? ` · ${t.tab_spawned_by_agent_hint}` : "")
            }
          >
            <span className="aiterm-tab-icon" style={{ position: "relative" }}>
              {getTabIcon(tab.type, tab.claudeBridge, tab.spawnedByAgent)}
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
              {/* Remote 指示器：右上角。不限分頁類型——terminal / database /
                  design / cross-db 四種都能擁有 Remote，指示器要跟著擁有者跑，
                  不然使用者在資料庫分頁開了 Remote 卻看不到任何訊號。
                  右上角在 mail 分頁是未讀數，但 mail 不能擁有 Remote（它沒用
                  那支 hook），所以不會撞；terminal 的左上（bridge）與右下
                  （attention）也各自錯開。 */}
              {tab.id === remoteTabId && (
                <span className="terminal-remote-badge" role="img" aria-label={t.term_remote_badge_label} />
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
