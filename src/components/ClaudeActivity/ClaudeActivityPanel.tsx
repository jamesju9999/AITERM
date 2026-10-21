import { useEffect, useMemo, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";
import { createPortal } from "react-dom";
import type { ActivitySnapshot } from "../../ipc/claudeActivity";
import { useLocale } from "../../contexts/LocaleContext";
import { DetailPane } from "./DetailPane";
import { foldFinished } from "./fold";
import { GraphCanvas } from "./GraphCanvas";
import { layoutGraph, ROOT_ID } from "./graphLayout";
import { formatTokens } from "./format";
import { tokenStats } from "./tokens";
import { attachTools, toolsToNodes } from "./toolNodes";
import { TimelineView } from "./TimelineView";
import "./ClaudeActivityPanel.css";

const COLLAPSED_KEY = "aiterm-claude-activity-collapsed";
const MODE_KEY = "aiterm-claude-activity-mode";
const TOOLS_KEY = "aiterm-claude-activity-tools";
const DETAIL_H_KEY = "aiterm-claude-activity-detail-h";
const DETAIL_OPEN_KEY = "aiterm-claude-activity-detail-open";
const DETAIL_MIN_H = 60;
const DETAIL_MAX_H = 600;

function loadDetailH(): number | null {
  try {
    const n = Number(localStorage.getItem(DETAIL_H_KEY));
    return Number.isFinite(n) && n >= DETAIL_MIN_H ? Math.min(n, DETAIL_MAX_H) : null;
  } catch {
    return null;
  }
}
function loadDetailOpen(): boolean {
  try {
    return localStorage.getItem(DETAIL_OPEN_KEY) !== "0";
  } catch {
    return true;
  }
}

type Mode = "graph" | "timeline";

function loadCollapsed(fill: boolean): boolean {
  // 獨佔整欄時收合只會留下一塊空白，所以沒存過偏好就預設展開；
  // 與建議側欄上下並排時才預設收合。使用者按過就以他的選擇為準。
  try {
    const saved = localStorage.getItem(COLLAPSED_KEY);
    if (saved === "0") return false;
    if (saved === "1") return true;
  } catch { /* ignore */ }
  return !fill;
}
function loadShowTools(): boolean {
  try {
    return localStorage.getItem(TOOLS_KEY) !== "0";
  } catch {
    return true;
  }
}
function loadMode(): Mode {
  try {
    return localStorage.getItem(MODE_KEY) === "timeline" ? "timeline" : "graph";
  } catch {
    return "graph";
  }
}

/** 有執行中的節點時每秒更新一次，讓耗時跳動；全部結束就停。 */
function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [active]);
  return now;
}

interface Props {
  snapshot: ActivitySnapshot;
  /** true＝獨佔側欄整欄；false＝與建議側欄上下並排時只佔上半部（有上限）。 */
  fill: boolean;
  /** 取工具明細要用；沒給就不顯示輸入輸出。 */
  sessionId?: string;
}

export function ClaudeActivityPanel({ snapshot, fill, sessionId }: Props) {
  const { t } = useLocale();
  const [collapsed, setCollapsed] = useState(() => loadCollapsed(fill));
  const [enlarged, setEnlarged] = useState(false);
  const [mode, setMode] = useState<Mode>(loadMode);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [pickedTurn, setPickedTurn] = useState<number | null>(null);
  const [showDone, setShowDone] = useState(false);
  const [showTools, setShowTools] = useState(loadShowTools);
  const [allTools, setAllTools] = useState(false);
  const [detailH, setDetailH] = useState<number | null>(loadDetailH);
  const [detailOpen, setDetailOpen] = useState(loadDetailOpen);
  const dockRef = useRef<HTMLDivElement>(null);

  const all = snapshot.nodes;
  const running = all.filter((n) => n.status === "running").length;
  const now = useNow(running > 0);

  // 依提問輪次分組：預設看最新一輪；挑到的輪次不存在了（例如離開 claude 後重來）就回到最新。
  // 輪次也要算進工具：沒有 subagent、只有主 Claude 的工具時，輪次只存在於工具上。
  const turns = useMemo(
    () => [...new Set([...all.map((n) => n.turn), ...snapshot.tools.map((c) => c.turn)])].sort((a, b) => a - b),
    [all, snapshot.tools],
  );
  const turn = pickedTurn !== null && turns.includes(pickedTurn) ? pickedTurn : turns[turns.length - 1] ?? 0;
  const turnIndex = Math.max(0, turns.indexOf(turn));
  const turnNodes = useMemo(() => all.filter((n) => n.turn === turn), [all, turn]);

  const folded = useMemo(() => foldFinished(turnNodes, t.claude_activity_done_group), [turnNodes, t]);
  const turnTools = useMemo(() => snapshot.tools.filter((c) => c.turn === turn), [snapshot.tools, turn]);
  const toolNodes = useMemo(
    () => (showTools ? toolsToNodes(turnTools, turn, { all: allTools, moreLabel: t.claude_activity_tools_more }) : []),
    [showTools, allTools, turnTools, turn, t],
  );
  // 時間軸不折疊，工具也全部列出（不套「最近 N 個」的視窗）。
  const timelineNodes = useMemo(() => (
    showTools ? [...turnNodes, ...toolsToNodes(turnTools, turn, { all: true, moreLabel: t.claude_activity_tools_more })] : turnNodes
  ), [showTools, turnNodes, turnTools, turn, t]);
  const visibleAgents = showDone ? turnNodes : folded.nodes;
  const visible = useMemo(() => attachTools(visibleAgents, toolNodes), [visibleAgents, toolNodes]);
  const graph = useMemo(() => layoutGraph(visible), [visible]);
  // 時間軸不折疊，所以選中的節點要在整輪裡找；圖則在折疊後看得到的節點裡找。
  const stats = useMemo(() => tokenStats(turnNodes), [turnNodes]);
  const rootSelected = mode === "graph" && selectedId === ROOT_ID;
  const selected = (mode === "timeline" ? timelineNodes : visible).find((n) => n.id === selectedId) ?? null;

  useEffect(() => {
    if (!enlarged) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setEnlarged(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [enlarged]);

  const toggleCollapsed = () => {
    const next = !collapsed;
    setCollapsed(next);
    try { localStorage.setItem(COLLAPSED_KEY, next ? "1" : "0"); } catch { /* ignore */ }
  };
  const toggleTools = () => {
    const next = !showTools;
    setShowTools(next);
    try { localStorage.setItem(TOOLS_KEY, next ? "1" : "0"); } catch { /* ignore */ }
  };
  const pickMode = (m: Mode) => {
    setMode(m);
    try { localStorage.setItem(MODE_KEY, m); } catch { /* ignore */ }
  };
  const toggleDetail = () => {
    const next = !detailOpen;
    setDetailOpen(next);
    try { localStorage.setItem(DETAIL_OPEN_KEY, next ? "1" : "0"); } catch { /* ignore */ }
  };
  const startResize = (e: ReactPointerEvent<HTMLDivElement>) => {
    const dock = dockRef.current;
    if (!dock || !detailOpen) return;
    const startY = e.clientY;
    const startH = dock.offsetHeight;
    let latest = startH;
    const move = (ev: PointerEvent) => {
      latest = Math.min(DETAIL_MAX_H, Math.max(DETAIL_MIN_H, startH + (startY - ev.clientY)));
      setDetailH(latest);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      try { localStorage.setItem(DETAIL_H_KEY, String(Math.round(latest))); } catch { /* ignore */ }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    e.preventDefault();
  };
  const stepTurn = (delta: number) => setPickedTurn(turns[Math.min(turns.length - 1, Math.max(0, turnIndex + delta))]);

  const header = (closeButton: boolean) => (
    <header className="aiterm-cact__header">
      <span className="aiterm-cact__title">{t.claude_activity_title}</span>
      <span className="aiterm-cact__count">{t.claude_activity_running(running)}</span>
      <span className="aiterm-cact__count">{t.claude_activity_total(turnNodes.length)}</span>
      {stats.total > 0 && <span className="aiterm-cact__count">{t.claude_activity_tokens_total(formatTokens(stats.total))}</span>}
      {closeButton ? (
        <button type="button" className="aiterm-cact__btn aiterm-cact__btn--first" onClick={() => setEnlarged(false)}>
          {t.claude_activity_close}
        </button>
      ) : (
        <>
          <button type="button" className="aiterm-cact__btn aiterm-cact__btn--first" onClick={() => setEnlarged(true)}>
            {t.claude_activity_enlarge}
          </button>
          <button type="button" className="aiterm-cact__btn" aria-expanded={!collapsed} onClick={toggleCollapsed}>
            {collapsed ? t.claude_activity_expand : t.claude_activity_collapse}
          </button>
        </>
      )}
    </header>
  );

  const toolbar = (
    <div className="aiterm-cact__toolbar">
      {snapshot.turn > (turns[turns.length - 1] ?? 0) && (
        <span className="aiterm-cact__hint">{t.claude_activity_latest_turn_empty}</span>
      )}
      {turns.length > 1 && (
        <span className="aiterm-cact__turns">
          <button type="button" className="aiterm-cact__btn" aria-label={t.claude_activity_prev_turn} disabled={turnIndex === 0} onClick={() => stepTurn(-1)}>‹</button>
          <span>{t.claude_activity_turn(turnIndex + 1, turns.length)}</span>
          <button type="button" className="aiterm-cact__btn" aria-label={t.claude_activity_next_turn} disabled={turnIndex === turns.length - 1} onClick={() => stepTurn(1)}>›</button>
        </span>
      )}
      <span className="aiterm-cact__seg" role="group">
        <button type="button" className={`aiterm-cact__btn${mode === "graph" ? " is-on" : ""}`} aria-pressed={mode === "graph"} onClick={() => pickMode("graph")}>
          {t.claude_activity_view_graph}
        </button>
        <button type="button" className={`aiterm-cact__btn${mode === "timeline" ? " is-on" : ""}`} aria-pressed={mode === "timeline"} onClick={() => pickMode("timeline")}>
          {t.claude_activity_view_timeline}
        </button>
      </span>
      {turnTools.length > 0 && (
        <button type="button" className="aiterm-cact__btn" onClick={toggleTools}>
          {showTools ? t.claude_activity_hide_tools : t.claude_activity_show_tools}
        </button>
      )}
      {folded.folded > 0 && (
        <button type="button" className="aiterm-cact__btn" onClick={() => setShowDone((v) => !v)}>
          {showDone ? t.claude_activity_hide_done : t.claude_activity_show_done(folded.folded)}
        </button>
      )}
    </div>
  );

  /** split＝放大時左右分欄：左邊圖／時間軸，右邊詳情；小面板則上下排，詳情可拖曳、可收合。 */
  const body = (split: boolean) => (
    <div className={`aiterm-cact__body${split ? " aiterm-cact__body--split" : ""}`}>
      <div className="aiterm-cact__main">
        {toolbar}
        {mode === "graph" ? (
          <GraphCanvas
            graph={graph}
            nodes={visible}
            now={now}
            selectedId={selectedId}
            onSelect={setSelectedId}
            runningCount={running}
            mainTool={snapshot.main_tool}
            mainToolCount={snapshot.main_tool_count}
            heaviestId={stats.heaviestId}
            onExpandDone={() => setShowDone(true)}
            onExpandTools={() => setAllTools(true)}
          />
        ) : (
          <TimelineView nodes={timelineNodes} now={now} selectedId={selectedId} onSelect={setSelectedId} />
        )}
      </div>
      <div
        className="aiterm-cact__dock"
        ref={split ? undefined : dockRef}
        style={!split && detailOpen && detailH != null ? { height: detailH, maxHeight: "none" } : undefined}
      >
        {!split && (
          <div
            className="aiterm-cact__handle"
            role="separator"
            aria-orientation="horizontal"
            aria-label={t.claude_activity_detail_resize}
            onPointerDown={startResize}
          >
            <button
              type="button"
              className="aiterm-cact__handle-btn"
              aria-expanded={detailOpen}
              aria-label={detailOpen ? t.claude_activity_detail_collapse : t.claude_activity_detail_expand}
              onPointerDown={(e) => e.stopPropagation()}
              onClick={toggleDetail}
            >
              {detailOpen ? "▾" : "▴"}
            </button>
          </div>
        )}
        {(split || detailOpen) && (
          <DetailPane
            node={selected}
            now={now}
            sessionId={sessionId}
            share={selected ? stats.share(selected) : null}
            main={rootSelected ? { count: snapshot.main_tool_count, recent: snapshot.main_recent } : null}
          />
        )}
      </div>
    </div>
  );

  return (
    <section
      className={`aiterm-cact${fill ? " aiterm-cact--fill" : ""}${collapsed ? " aiterm-cact--collapsed" : ""}`}
      aria-label={t.claude_activity_title}
    >
      {header(false)}
      {!collapsed && body(false)}
      {enlarged &&
        createPortal(
          <div className="aiterm-cact-overlay" onClick={() => setEnlarged(false)}>
            <div
              className="aiterm-cact aiterm-cact--fill aiterm-cact--overlay"
              role="dialog"
              aria-label={t.claude_activity_title}
              onClick={(e) => e.stopPropagation()}
            >
              {header(true)}
              {body(true)}
            </div>
          </div>,
          document.body,
        )}
    </section>
  );
}
