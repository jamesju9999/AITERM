import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
import { createPortal } from "react-dom";
import { confirm } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import {
  abortAgentMerge,
  assignAgent,
  createAgent,
  deleteAgent,
  listAgents,
  mergeAgent,
  overrideModule,
  resolveAgent,
  updateAgent,
  type AgentKind,
  type AgentRow,
} from "../../ipc/agents";
import { listProjects, type ProjectInfo } from "../../ipc/projects";
import {
  getTaskBoardConfig,
  listTasks,
  onTasksUpdated,
  type MergeOutcome,
  type TaskWithAttachments,
} from "../../ipc/tasks";
import { unlistenOnCleanup } from "../../lib/eventSubscription";
import { TaskEditorDialog } from "../TaskBoard/TaskEditorDialog";
import { TranscriptDialog } from "../TaskBoard/TranscriptDialog";
import "../TaskBoard/index.css";
import { AgentConfigDialog, type AgentConfigValues } from "./AgentConfigDialog";
import { AgentNode } from "./AgentNode";
import { deriveAgentStatus } from "./deriveAgentStatus";
import { deriveModuleState } from "./deriveModuleState";
import { EdgeLayer } from "./EdgeLayer";
import { computeEdges, diffPulses, edgeActivity, type Pulse } from "./edges";
import "./index.css";

/** 跟看板、TabBar 同一個門檻：小於這個位移是點擊，不是拖曳。 */
const DRAG_THRESHOLD_PX = 4;
const PROJECT_KEY = "aiterm.agentCanvas.project";

type DragKind = "pool" | "module" | "agent";

interface DragState {
  kind: DragKind;
  /** module = 卡片 id；agent = agent id；pool = Agent 種類。 */
  id: string | null;
  startX: number;
  startY: number;
  started: boolean;
  /** agent 拖曳：滑鼠相對於卡片左上角的偏移。 */
  offX: number;
  offY: number;
}

interface DropTarget {
  kind: "agent" | "library" | "canvas";
  agentId?: string;
}

function readStoredProject(): string | null {
  try {
    return localStorage.getItem(PROJECT_KEY);
  } catch {
    return null;
  }
}

function storeProject(id: string) {
  try {
    localStorage.setItem(PROJECT_KEY, id);
  } catch {
    // 私密視窗等情況存不了，下次就回到第一個專案，無妨。
  }
}

function errText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

/** 拖曳放開那一點底下是什麼。用 `elementFromPoint` 而不是 HTML5 的 drop
 *  事件：Tauri 的 dragDropEnabled 會吃掉原生拖放（看板踩過）。 */
function hitTest(x: number, y: number): DropTarget | null {
  const el = document.elementFromPoint(x, y);
  if (!el) return null;
  const agentEl = el.closest<HTMLElement>("[data-agent-id]");
  if (agentEl) return { kind: "agent", agentId: agentEl.dataset.agentId };
  if (el.closest('[data-drop="library"]')) return { kind: "library" };
  if (el.closest('[data-drop="canvas"]')) return { kind: "canvas" };
  return null;
}

export function AgentCanvasView({ isActive }: { isActive: boolean }) {
  const { t } = useLocale();
  const [projects, setProjects] = useState<ProjectInfo[] | null>(null);
  const [projectId, setProjectId] = useState<string | null>(null);
  const [agents, setAgents] = useState<AgentRow[]>([]);
  const [tasks, setTasks] = useState<TaskWithAttachments[]>([]);
  const [cap, setCap] = useState<number | null>(null);
  const [nowSecs, setNowSecs] = useState(() => Math.floor(Date.now() / 1000));
  const [config, setConfig] = useState<{ kind: AgentKind; agent: AgentRow | null; x: number; y: number } | null>(null);
  /** 一次性的交接光點；每顆約 1.2 秒後移除。 */
  const [pulses, setPulses] = useState<Pulse[]>([]);
  const prevTasks = useRef<TaskWithAttachments[] | null>(null);
  const [newModule, setNewModule] = useState(false);
  const [transcriptFor, setTranscriptFor] = useState<string | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [merge, setMerge] = useState<Record<string, { text: string; conflict: boolean }>>({});
  /** 拖曳 Agent 卡片時的即時位置。放開才寫回資料庫。 */
  const [movePos, setMovePos] = useState<{ id: string; x: number; y: number } | null>(null);
  const [ghost, setGhost] = useState<{ kind: DragKind; id: string | null; x: number; y: number } | null>(null);
  const [hover, setHover] = useState<DropTarget | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const canvasRef = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);

  // 專案清單。
  useEffect(() => {
    mounted.current = true;
    listProjects()
      .then((ps) => {
        if (!mounted.current) return;
        const ok = ps.filter((p) => p.status === "ok");
        setProjects(ok);
        const stored = readStoredProject();
        setProjectId((cur) => cur ?? (ok.find((p) => p.id === stored) ?? ok[0])?.id ?? null);
      })
      .catch((e) => setToast(errText(e)));
    getTaskBoardConfig()
      .then((c) => mounted.current && setCap(c.max_concurrent))
      .catch(() => {});
    return () => {
      mounted.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    if (!projectId) return;
    try {
      const [a, ts] = await Promise.all([listAgents(projectId), listTasks(projectId)]);
      if (!mounted.current) return;
      setAgents(a);
      setTasks(ts);
    } catch (e) {
      if (mounted.current) setToast(errText(e));
    }
  }, [projectId]);

  // 載入＋訂閱寫在 effect 裡（而不是呼叫 refresh），setState 只發生在
  // 非同步回呼中；`alive` 擋掉切換專案後舊請求晚到的結果。
  useEffect(() => {
    if (!projectId) return;
    let alive = true;
    prevTasks.current = null;
    const load = () => {
      Promise.all([listAgents(projectId), listTasks(projectId)])
        .then(([a, ts]) => {
          if (!alive) return;
          const fresh = diffPulses(prevTasks.current, ts, a);
          prevTasks.current = ts;
          setAgents(a);
          setTasks(ts);
          if (fresh.length) {
            setPulses((cur) => [...cur.filter((p) => !fresh.some((f) => f.key === p.key)), ...fresh]);
            window.setTimeout(() => setPulses((cur) => cur.filter((p) => !fresh.includes(p))), 1300);
          }
        })
        .catch((e) => alive && setToast(errText(e)));
    };
    load();
    const unlisten = unlistenOnCleanup(onTasksUpdated(load), "tasks-updated");
    return () => {
      alive = false;
      unlisten();
    };
  }, [projectId]);

  const anyRunning = tasks.some((x) => x.status === "running" && x.agent_id);
  // 經過時間每秒更新——只在看得到、而且真的有東西在跑的時候。
  useEffect(() => {
    if (!isActive || !anyRunning) return;
    const h = window.setInterval(() => setNowSecs(Math.floor(Date.now() / 1000)), 1000);
    return () => window.clearInterval(h);
  }, [isActive, anyRunning]);

  useEffect(() => {
    if (!toast) return;
    const h = window.setTimeout(() => setToast(null), 3500);
    return () => window.clearTimeout(h);
  }, [toast]);

  const run = useCallback(
    async (fn: () => Promise<unknown>) => {
      try {
        await fn();
      } catch (e) {
        setToast(errText(e));
      }
      void refresh();
    },
    [refresh],
  );

  const library = useMemo(
    () =>
      tasks
        .filter((x) => x.status === "planning" && !x.agent_id && x.card_kind === "work")
        .sort((a, b) => a.sort_order - b.sort_order),
    [tasks],
  );
  const cardsOf = useCallback(
    (agentId: string) => tasks.filter((x) => x.agent_id === agentId),
    [tasks],
  );
  const agentName = useCallback((id: string | null) => agents.find((a) => a.id === id)?.name ?? "?", [agents]);

  const activity = useMemo(() => {
    const ev: { at: number; text: string; kind: "start" | "ok" | "bad" }[] = [];
    for (const c of tasks) {
      if (!c.agent_id) continue;
      const who = agentName(c.agent_id);
      if (c.dispatched_at) ev.push({ at: c.dispatched_at, text: t.ac_log_started(who, c.title), kind: "start" });
      if (c.finished_at)
        ev.push(
          c.outcome === "success"
            ? { at: c.finished_at, text: t.ac_log_finished(who, c.title), kind: "ok" }
            : { at: c.finished_at, text: t.ac_log_failed(who, c.title), kind: "bad" },
        );
    }
    return ev.sort((a, b) => b.at - a.at).slice(0, 40);
  }, [tasks, agentName, t]);

  // ── 拖曳 ───────────────────────────────────────────────────────────────

  const canvasPoint = (clientX: number, clientY: number) => {
    const el = canvasRef.current;
    if (!el) return { x: 0, y: 0 };
    const r = el.getBoundingClientRect();
    return { x: clientX - r.left + el.scrollLeft, y: clientY - r.top + el.scrollTop };
  };

  const beginDrag = (e: ReactMouseEvent, kind: DragKind, id: string | null) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button") && kind === "agent") return;
    e.preventDefault();
    let offX = 0;
    let offY = 0;
    if (kind === "agent" && id) {
      const a = agents.find((x) => x.id === id);
      const p = canvasPoint(e.clientX, e.clientY);
      offX = p.x - (a?.canvas_x ?? 0);
      offY = p.y - (a?.canvas_y ?? 0);
    }
    dragRef.current = { kind, id, startX: e.clientX, startY: e.clientY, started: false, offX, offY };
  };

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      const d = dragRef.current;
      if (!d) return;
      if (!d.started) {
        if (Math.hypot(e.clientX - d.startX, e.clientY - d.startY) < DRAG_THRESHOLD_PX) return;
        d.started = true;
      }
      if (d.kind === "agent" && d.id) {
        const p = canvasPoint(e.clientX, e.clientY);
        setMovePos({ id: d.id, x: Math.max(0, p.x - d.offX), y: Math.max(0, p.y - d.offY) });
        return;
      }
      setGhost({ kind: d.kind, id: d.id, x: e.clientX, y: e.clientY });
      setHover(hitTest(e.clientX, e.clientY));
    };
    const onUp = (e: MouseEvent) => {
      const d = dragRef.current;
      dragRef.current = null;
      setGhost(null);
      setHover(null);
      if (!d || !d.started || !projectId) return;
      if (d.kind === "agent" && d.id) {
        const a = agents.find((x) => x.id === d.id);
        const p = canvasPoint(e.clientX, e.clientY);
        const x = Math.max(0, p.x - d.offX);
        const y = Math.max(0, p.y - d.offY);
        if (a) {
          // 先在畫面上停在新位置，寫回完成後 refresh 會帶回同一個值。
          setAgents((prev) => prev.map((g) => (g.id === a.id ? { ...g, canvas_x: x, canvas_y: y } : g)));
          void run(() =>
            updateAgent(projectId, { id: a.id, name: a.name, role: a.role, prompt_prefix: a.prompt_prefix, canvas_x: x, canvas_y: y }),
          );
        }
        setMovePos(null);
        return;
      }
      const target = hitTest(e.clientX, e.clientY);
      if (d.kind === "pool") {
        if (target?.kind !== "canvas") return;
        const p = canvasPoint(e.clientX, e.clientY);
        setConfig({ kind: (d.id ?? "coder") as AgentKind, agent: null, x: Math.max(8, p.x - 120), y: Math.max(8, p.y - 24) });
        return;
      }
      if (d.kind === "module" && d.id) {
        const card = tasks.find((x) => x.id === d.id);
        if (!card) return;
        if (target?.kind === "agent" && target.agentId) {
          const a = agents.find((x) => x.id === target.agentId);
          if (!a || a.kind !== "coder") {
            setToast(t.ac_only_coder);
            return;
          }
          if (card.agent_id === a.id) return;
          void run(() => assignAgent(projectId, card.id, a.id));
        } else if (target?.kind === "library" && card.agent_id) {
          void run(() => assignAgent(projectId, card.id, null));
        }
      }
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [agents, tasks, projectId, run, t]);

  // ── 動作 ───────────────────────────────────────────────────────────────

  const submitConfig = async (v: AgentConfigValues) => {
    if (!projectId || !config) return;
    if (config.agent) {
      const a = config.agent;
      await updateAgent(projectId, {
        id: a.id,
        name: v.name,
        role: v.role,
        prompt_prefix: v.prompt_prefix,
        canvas_x: a.canvas_x,
        canvas_y: a.canvas_y,
        produces_spec: v.produces_spec,
      });
    } else {
      await createAgent(projectId, {
        kind: config.kind,
        name: v.name,
        role: v.role,
        prompt_prefix: v.prompt_prefix,
        project_dir: v.project_dir,
        canvas_x: config.x,
        canvas_y: config.y,
        produces_spec: v.produces_spec,
      });
    }
    setConfig(null);
    void refresh();
  };

  const removeAgent = async (a: AgentRow) => {
    if (!projectId) return;
    // window.confirm 在 Tauri webview 裡沒有實作，要用 plugin-dialog。
    const ok = await confirm(t.ac_delete_confirm(a.name), { kind: "warning" });
    if (!ok) return;
    void run(() => deleteAgent(projectId, a.id));
  };

  const describeMerge = (o: MergeOutcome): { text: string; conflict: boolean } => {
    switch (o.status) {
      case "merged":
      case "merged_but_not_cleaned":
        return { text: t.ac_merge_done, conflict: false };
      case "conflict":
        return { text: t.ac_merge_conflict(o.files.length), conflict: true };
      case "blocked":
        return {
          text: o.reason === "dirty_base" ? t.ac_merge_blocked_dirty : t.ac_merge_blocked_in_progress,
          conflict: o.reason === "merge_in_progress",
        };
    }
  };

  const doMerge = (a: AgentRow) => {
    if (!projectId) return;
    void run(async () => {
      const o = await mergeAgent(projectId, a.id);
      setMerge((m) => ({ ...m, [a.id]: describeMerge(o) }));
    });
  };

  const doAbortMerge = (a: AgentRow) => {
    if (!projectId) return;
    void run(async () => {
      await abortAgentMerge(projectId, a.id);
      setMerge((m) => {
        const next = { ...m };
        delete next[a.id];
        return next;
      });
    });
  };

  const openTab = (tabId: string) => {
    window.dispatchEvent(new CustomEvent("aiterm:focus-tab", { detail: { tabId } }));
  };

  const dropStateFor = (a: AgentRow): "ok" | "bad" | null => {
    if (!ghost || ghost.kind !== "module" || hover?.kind !== "agent" || hover.agentId !== a.id) return null;
    return a.kind === "coder" ? "ok" : "bad";
  };

  const ghostCard = ghost?.kind === "module" ? tasks.find((x) => x.id === ghost.id) : undefined;
  const coderIds = useMemo(() => new Set(agents.filter((a) => a.kind === "coder").map((a) => a.id)), [agents]);
  const hasReviewer = agents.some((a) => a.kind === "reviewer");
  const shownAgents = agents.map((a) => (movePos?.id === a.id ? { ...a, canvas_x: movePos.x, canvas_y: movePos.y } : a));
  const edges = computeEdges(agents);
  const active = edgeActivity(edges, agents, tasks);
  const placed = (k: AgentKind) => agents.some((a) => a.kind === k);
  const poolLabel = (k: AgentKind) => (k === "reviewer" ? t.ac_reviewer : k === "orchestrator" ? t.ac_orchestrator : t.ac_coder);
  const transcriptCard = transcriptFor ? tasks.find((x) => x.id === transcriptFor) : undefined;
  const defaultName = (k: AgentKind) =>
    k === "coder" ? `${t.ac_coder}${agents.filter((a) => a.kind === "coder").length + 1}` : poolLabel(k);

  // ── 畫面 ───────────────────────────────────────────────────────────────

  if (projects !== null && projects.length === 0) {
    return (
      <div className="task-board agent-canvas">
        <div className="ac-empty-state">{t.ac_no_projects}</div>
      </div>
    );
  }

  return (
    <div className="task-board agent-canvas">
      <div className="ac-top">
        <h1 className="ac-title">
          <span className="ac-dot" />
          {t.agent_canvas_tab}
        </h1>
        <label className="ac-ctl">
          {t.ac_project}
          <select
            value={projectId ?? ""}
            onChange={(e) => {
              setProjectId(e.target.value);
              storeProject(e.target.value);
              setMerge({});
            }}
          >
            {(projects ?? []).map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
        <span className="ac-spacer" />
        {cap !== null && <span className="ac-ctl">{t.ac_cap(cap)}</span>}
      </div>

      <div className="ac-pool">
        <span className="ac-pool-label">{t.ac_pool}</span>
        {(["coder", "reviewer", "orchestrator"] as const).map((k) => {
          const disabled = k !== "coder" && placed(k);
          return (
            <div
              key={k}
              className={`ac-pool-item ac-pool-item--${k}${disabled ? " ac-pool-item--disabled" : ""}`}
              data-testid={`pool-${k}`}
              aria-disabled={disabled}
              title={disabled ? t.ac_already_placed : undefined}
              onMouseDown={disabled ? undefined : (e) => beginDrag(e, "pool", k)}
            >
              <span className="ac-agent-ico">{k === "coder" ? "</>" : k === "reviewer" ? "✓?" : "⑂"}</span>
              <div>
                {poolLabel(k)}
                <small>{disabled ? t.ac_already_placed : k === "coder" ? t.ac_coder_hint : k === "reviewer" ? t.ac_reviewer_hint : t.ac_orchestrator_hint}</small>
              </div>
            </div>
          );
        })}
        <span className="ac-pool-hint">{t.ac_pool_hint}</span>
      </div>

      <div className="ac-main">
        <aside className={`ac-lib${hover?.kind === "library" ? " ac-lib--drop" : ""}`} data-drop="library">
          <h2>{t.ac_library}</h2>
          <div className="ac-lib-sub">{t.ac_library_sub}</div>
          <div className="ac-lib-list">
            {library.length === 0 && <div className="ac-lib-empty">{t.ac_library_empty}</div>}
            {library.map((c) => (
              <div
                key={c.id}
                className="ac-module"
                data-testid={`module-${c.id}`}
                onMouseDown={(e) => beginDrag(e, "module", c.id)}
                title={c.body}
              >
                <span className="ac-module-title">{c.title}</span>
              </div>
            ))}
          </div>
          <button className="aiterm-btn aiterm-btn--secondary ac-add" disabled={!projectId} onClick={() => setNewModule(true)}>
            {t.ac_add_module}
          </button>
        </aside>

        <section className={`ac-canvas${hover?.kind === "canvas" && ghost?.kind === "pool" ? " ac-canvas--drop" : ""}`} data-drop="canvas" ref={canvasRef}>
          {agents.length === 0 && <div className="ac-canvas-empty">{t.ac_empty_canvas}</div>}
          <EdgeLayer containerRef={canvasRef} agents={shownAgents} edges={edges} active={active} pulses={pulses} revision={tasks} />
          {shownAgents.map((shown) => {
            const a = agents.find((x) => x.id === shown.id) as AgentRow;
            return (
              <AgentNode
                key={a.id}
                agent={shown}
                tasks={tasks}
                agentName={agentName}
                coderIds={coderIds}
                hasReviewer={hasReviewer}
                status={deriveAgentStatus(
                  a,
                  cardsOf(a.id),
                  a.kind === "coder"
                    ? cardsOf(a.id).filter((c) => c.card_kind === "work").map((m) => deriveModuleState(m, tasks, hasReviewer))
                    : a.kind === "orchestrator"
                      ? tasks
                          .filter((c) => c.card_kind === "work" && c.agent_id && coderIds.has(c.agent_id))
                          .map((m) => deriveModuleState(m, tasks, hasReviewer))
                      : undefined,
                )}
                nowSecs={nowSecs}
                dropState={dropStateFor(a)}
                mergeMessage={merge[a.id]?.text ?? null}
                mergeConflict={merge[a.id]?.conflict ?? false}
                onHeaderMouseDown={(e) => beginDrag(e, "agent", a.id)}
                onChipMouseDown={(e, taskId) => {
                  e.stopPropagation();
                  beginDrag(e, "module", taskId);
                }}
                onEdit={() => setConfig({ kind: a.kind, agent: a, x: a.canvas_x, y: a.canvas_y })}
                onDelete={() => void removeAgent(a)}
                onRetry={() => projectId && void run(() => resolveAgent(projectId, a.id, "retry"))}
                onSkip={() => projectId && void run(() => resolveAgent(projectId, a.id, "skip"))}
                onTranscript={setTranscriptFor}
                onOpenTab={openTab}
                onMerge={() => doMerge(a)}
                onAbortMerge={() => doAbortMerge(a)}
                onRelease={(moduleId) => projectId && void run(() => overrideModule(projectId, moduleId))}
              />
            );
          })}
        </section>
      </div>

      <div className="ac-log">
        <div className="ac-log-title">{t.ac_log_title}</div>
        <div className="ac-log-lines">
          {activity.length === 0 && <div className="ac-log-empty">{t.ac_log_empty}</div>}
          {activity.map((ev, i) => (
            <div key={i} className={`ac-log-line ac-log-line--${ev.kind}`}>
              <span className="ac-log-time">{new Date(ev.at * 1000).toLocaleTimeString()}</span>
              {ev.text}
            </div>
          ))}
        </div>
      </div>

      {toast && (
        <div className="ac-toast" role="status">
          {toast}
        </div>
      )}

      {ghost &&
        createPortal(
          <div className="task-card-ghost ac-ghost" style={{ transform: `translate(${ghost.x + 10}px, ${ghost.y + 10}px)` }}>
            {ghost.kind === "pool" ? poolLabel((ghost.id ?? "coder") as AgentKind) : (ghostCard?.title ?? "")}
          </div>,
          document.body,
        )}

      {config && projectId && (
        <AgentConfigDialog
          projectId={projectId}
          kind={config.kind}
          agent={config.agent}
          defaultName={defaultName(config.kind)}
          onCancel={() => setConfig(null)}
          onSubmit={submitConfig}
        />
      )}
      {newModule && projectId && (
        <TaskEditorDialog
          projectId={projectId}
          card={null}
          onClose={() => setNewModule(false)}
          onSaved={() => {
            setNewModule(false);
            void refresh();
          }}
        />
      )}
      {transcriptCard && projectId && (
        <TranscriptDialog
          projectId={projectId}
          taskId={transcriptCard.id}
          body={transcriptCard.body}
          onClose={() => setTranscriptFor(null)}
        />
      )}
    </div>
  );
}
