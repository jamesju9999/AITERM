import { useEffect, useMemo, useState } from "react";
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

type Mode = "graph" | "timeline";

function loadCollapsed(): boolean {
  try {
    // 沒存過＝預設收合；使用者展開過才記住展開。
    return localStorage.getItem(COLLAPSED_KEY) !== "0";
  } catch {
    return true;
  }
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
  const [collapsed, setCollapsed] = useState(loadCollapsed);
  const [enlarged, setEnlarged] = useState(false);
  const [mode, setMode] = useState<Mode>(loadMode);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [pickedTurn, setPickedTurn] = useState<number | null>(null);
  const [showDone, setShowDone] = useState(false);
  const [showTools, setShowTools] = useState(loadShowTools);
  const [allTools, setAllTools] = useState(false);

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
  const visibleAgents = showDone ? turnNodes : folded.nodes;
  const visible = useMemo(() => attachTools(visibleAgents, toolNodes), [visibleAgents, toolNodes]);
  const graph = useMemo(() => layoutGraph(visible), [visible]);
  // 時間軸不折疊，所以選中的節點要在整輪裡找；圖則在折疊後看得到的節點裡找。
  const stats = useMemo(() => tokenStats(turnNodes), [turnNodes]);
  const rootSelected = mode === "graph" && selectedId === ROOT_ID;
  const selected = (mode === "timeline" ? turnNodes : visible).find((n) => n.id === selectedId) ?? null;

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

  const body = (
    <>
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
        <TimelineView nodes={turnNodes} now={now} selectedId={selectedId} onSelect={setSelectedId} />
      )}
      <DetailPane
        node={selected}
        now={now}
        sessionId={sessionId}
        share={selected ? stats.share(selected) : null}
        main={rootSelected ? { count: snapshot.main_tool_count, recent: snapshot.main_recent } : null}
      />
    </>
  );

  return (
    <section
      className={`aiterm-cact${fill ? " aiterm-cact--fill" : ""}${collapsed ? " aiterm-cact--collapsed" : ""}`}
      aria-label={t.claude_activity_title}
    >
      {header(false)}
      {!collapsed && body}
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
              {body}
            </div>
          </div>,
          document.body,
        )}
    </section>
  );
}
