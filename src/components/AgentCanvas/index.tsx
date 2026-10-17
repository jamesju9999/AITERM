import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from "react";
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
  openRepo,
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
  listTokenUsage,
  markTaskDone,
  onTasksUpdated,
  type MergeOutcome,
  type TaskWithAttachments,
  type TokenUsage,
} from "../../ipc/tasks";
import { unlistenOnCleanup } from "../../lib/eventSubscription";
import { TaskEditorDialog } from "../TaskBoard/TaskEditorDialog";
import { TranscriptDialog } from "../TaskBoard/TranscriptDialog";
import "../TaskBoard/index.css";
import { AgentConfigDialog, type AgentConfigValues } from "./AgentConfigDialog";
import { AgentNode } from "./AgentNode";
import { localizedCardTitle } from "./cardTitle";
import { deriveAgentStatus } from "./deriveAgentStatus";
import { deriveModuleState } from "./deriveModuleState";
import { EdgeLayer } from "./EdgeLayer";
import { WorkflowIcon } from "../Icons";
import { computeEdges, diffPulses, edgeActivity, type Pulse } from "./edges";
import { idList } from "./prerequisites";
import { QuestionInbox } from "./QuestionInbox";
import { answerQuestion, listQuestions, onQuestionsUpdated, type QuestionView } from "../../ipc/questions";
import { projectSummary, type ProjectSummary } from "./projectSummary";
import { formatTokens, sumUsage, totalTokens, usageBreakdown } from "./tokenUsage";
import "./index.css";

/** 跟看板、TabBar 同一個門檻：小於這個位移是點擊，不是拖曳。 */
const DRAG_THRESHOLD_PX = 4;
const PROJECT_KEY = "aiterm.agentCanvas.project";
const LAYOUT_KEY = "aiterm.agentCanvas.layout";
const MIN_ZOOM = 0.3;
const MAX_ZOOM = 1.5;
/** 量不到節點尺寸時（還沒畫出來）的估計值。 */
const NODE_W = 248;
const NODE_H = 140;
/** 桌布在最遠的節點之外再留多少空間，讓人能把節點往外拖。 */
const WORLD_MARGIN = 600;

interface Layout {
  libCollapsed: boolean;
  logCollapsed: boolean;
  logHeight: number;
}
const DEFAULT_LAYOUT: Layout = { libCollapsed: false, logCollapsed: false, logHeight: 120 };

function readLayout(): Layout {
  try {
    const raw = localStorage.getItem(LAYOUT_KEY);
    return raw ? { ...DEFAULT_LAYOUT, ...(JSON.parse(raw) as Partial<Layout>) } : DEFAULT_LAYOUT;
  } catch {
    return DEFAULT_LAYOUT;
  }
}

function storeLayout(l: Layout) {
  try {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify(l));
  } catch {
    // 存不了就只在這次有效。
  }
}

const clampZoom = (z: number) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, z));

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
  /** 每張卡的 token 用量；記著是哪個專案的，切專案時舊的不會被拿來用。 */
  const [usageOf, setUsageOf] = useState<{ projectId: string; map: Record<string, TokenUsage> } | null>(null);
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
  const [zoom, setZoom] = useState(1);
  /** 事件處理器與拖曳用的最新縮放值（避免閉包拿到舊值）。 */
  const zoomRef = useRef(1);
  useLayoutEffect(() => {
    zoomRef.current = zoom;
  }, [zoom]);
  /** 縮放後要把哪個桌布座標留在畫面上的哪個位置（縮放後的 layout effect 套用）。 */
  const zoomAnchor = useRef<{ wx: number; wy: number; vx: number; vy: number } | null>(null);
  /** 已經自動「全部顯示」過的專案——每個專案打開時做一次。 */
  const fittedFor = useRef<string | null>(null);
  const [layout, setLayoutState] = useState<Layout>(readLayout);
  /** 所有專案還沒回答的 Agent 問題（收件匣）。 */
  const [questions, setQuestions] = useState<QuestionView[]>([]);
  const [inbox, setInbox] = useState<{ open: boolean; focus: number | null }>({ open: false, focus: null });
  const setLayout = (patch: Partial<Layout>) =>
    setLayoutState((cur) => {
      const next = { ...cur, ...patch };
      storeLayout(next);
      return next;
    });

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
      listTokenUsage(projectId)
        .then((map) => alive && setUsageOf({ projectId, map }))
        .catch(() => {});
    };
    load();
    const unlisten = unlistenOnCleanup(onTasksUpdated(load), "tasks-updated");
    return () => {
      alive = false;
      unlisten();
    };
  }, [projectId]);

  // 收件匣：有新問題、回答、或卡片結束讓問題作廢時重抓；卡片狀態變了也重抓（可能有問題作廢）。
  useEffect(() => {
    let alive = true;
    const load = () => {
      listQuestions()
        .then((qs) => alive && setQuestions(qs))
        .catch(() => {});
    };
    load();
    const stopQ = unlistenOnCleanup(onQuestionsUpdated(load), "questions-updated");
    const stopT = unlistenOnCleanup(onTasksUpdated(load), "tasks-updated");
    return () => {
      alive = false;
      stopQ();
      stopT();
    };
  }, []);

  const doAnswer = async (q: QuestionView, answer: string) => {
    try {
      await answerQuestion(q.project_id, q.id, answer);
      setQuestions((cur) => cur.filter((x) => !(x.project_id === q.project_id && x.id === q.id)));
    } catch (e) {
      setToast(errText(e));
    }
  };

  const anyRunning = tasks.some((x) => x.status === "running" && x.agent_id);
  // 經過時間每秒更新——只在看得到、而且真的有東西在跑的時候。
  useEffect(() => {
    if (!isActive || !anyRunning) return;
    const h = window.setInterval(() => setNowSecs(Math.floor(Date.now() / 1000)), 1000);
    return () => window.clearInterval(h);
  }, [isActive, anyRunning]);
  // 執行中卡片的用量一直在長：看得到的時候每 15 秒重抓一次。
  useEffect(() => {
    if (!isActive || !anyRunning || !projectId) return;
    const h = window.setInterval(() => {
      listTokenUsage(projectId)
        .then((map) => setUsageOf({ projectId, map }))
        .catch(() => {});
    }, 15_000);
    return () => window.clearInterval(h);
  }, [isActive, anyRunning, projectId]);
  const usage = useMemo(() => (usageOf && usageOf.projectId === projectId ? usageOf.map : {}), [usageOf, projectId]);
  const projectUsage = useMemo(() => sumUsage(tasks, usage), [tasks, usage]);

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
    const ev: { at: number; text: string; kind: "start" | "ok" | "bad" | "dispatch" }[] = [];
    const titleOf = (id: string) => tasks.find((x) => x.id === id)?.title;
    for (const c of tasks) {
      if (!c.agent_id) continue;
      const who = agentName(c.agent_id);
      const title = localizedCardTitle(c, t);
      // 派工過程（由系統決定，見 docs/superpowers/specs/2026-10-16-agent-task-dependencies-design.md §7）。
      if (c.from_spec && c.card_kind === "work") {
        // created_at 是 SQLite 的 datetime('now')：UTC、沒有時區標記。
        const created = Date.parse(`${c.created_at.replace(" ", "T")}Z`) / 1000;
        if (Number.isFinite(created)) {
          ev.push({ at: created, text: t.ac_log_dispatched(title, agentName(c.assigned_from ?? c.agent_id)), kind: "dispatch" });
        }
      }
      if (c.assigned_from && c.assigned_at) {
        ev.push({ at: c.assigned_at, text: t.ac_log_reassigned(title, agentName(c.assigned_from), who), kind: "dispatch" });
      }
      const merged = idList(c.premerged).map(titleOf).filter((x): x is string => !!x);
      if (merged.length && c.dispatched_at) {
        ev.push({ at: c.dispatched_at, text: t.ac_log_premerged(merged.join("、"), who), kind: "dispatch" });
      }
      if (c.dispatched_at) ev.push({ at: c.dispatched_at, text: t.ac_log_started(who, title), kind: "start" });
      if (c.finished_at)
        ev.push(
          c.outcome === "success"
            ? { at: c.finished_at, text: t.ac_log_finished(who, title), kind: "ok" }
            : { at: c.finished_at, text: t.ac_log_failed(who, title), kind: "bad" },
        );
    }
    return ev.sort((a, b) => b.at - a.at).slice(0, 40);
  }, [tasks, agentName, t]);

  // ── 拖曳 ───────────────────────────────────────────────────────────────

  const canvasPoint = (clientX: number, clientY: number) => {
    const el = canvasRef.current;
    if (!el) return { x: 0, y: 0 };
    const r = el.getBoundingClientRect();
    const z = zoomRef.current;
    return { x: (clientX - r.left + el.scrollLeft) / z, y: (clientY - r.top + el.scrollTop) / z };
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
        model: v.model,
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
        model: v.model,
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

  const markDone = async (taskId: string) => {
    if (!projectId) return;
    // window.confirm 在 Tauri webview 裡沒有實作，要用 plugin-dialog。
    if (!(await confirm(t.board_mark_done_confirm))) return;
    void run(() => markTaskDone(projectId, taskId));
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

  // ── 縮放與「全部顯示」 ───────────────────────────────────────────────────

  /** 節點在桌布座標裡的外框（尺寸從 DOM 量，量不到用估計值）。 */
  const nodeBounds = useCallback(() => {
    const root = canvasRef.current;
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const a of agents) {
      const el = root?.querySelector<HTMLElement>(`[data-agent-id="${a.id}"]`);
      minX = Math.min(minX, a.canvas_x);
      minY = Math.min(minY, a.canvas_y);
      maxX = Math.max(maxX, a.canvas_x + (el?.offsetWidth || NODE_W));
      maxY = Math.max(maxY, a.canvas_y + (el?.offsetHeight || NODE_H));
    }
    return { minX, minY, maxX, maxY };
  }, [agents]);

  /** 縮放並捲動到所有節點都看得到。畫面還沒有尺寸（分頁隱藏中）時回 false。 */
  const fitToView = useCallback((): boolean => {
    const el = canvasRef.current;
    if (!el || agents.length === 0 || !el.clientWidth || !el.clientHeight) return false;
    const { minX, minY, maxX, maxY } = nodeBounds();
    const pad = 32;
    const z = clampZoom(
      Math.min(1, (el.clientWidth - pad * 2) / (maxX - minX), (el.clientHeight - pad * 2) / (maxY - minY)),
    );
    // 把節點群的中心放到畫面中心。
    zoomAnchor.current = {
      wx: (minX + maxX) / 2,
      wy: (minY + maxY) / 2,
      vx: el.clientWidth / 2,
      vy: el.clientHeight / 2,
    };
    setZoom(z);
    return true;
  }, [agents.length, nodeBounds]);

  /** 以畫面上的 (vx, vy) 為中心縮放。 */
  const zoomAround = useCallback((next: number, vx?: number, vy?: number) => {
    const el = canvasRef.current;
    if (!el) return;
    const z = zoomRef.current;
    const cx = vx ?? el.clientWidth / 2;
    const cy = vy ?? el.clientHeight / 2;
    zoomAnchor.current = { wx: (el.scrollLeft + cx) / z, wy: (el.scrollTop + cy) / z, vx: cx, vy: cy };
    setZoom(clampZoom(next));
  }, []);

  // 縮放後依錨點捲動。每次 zoom 改變都要套用，即使值沒變（全部顯示也要捲到位）。
  useLayoutEffect(() => {
    const el = canvasRef.current;
    const a = zoomAnchor.current;
    if (!el || !a) return;
    zoomAnchor.current = null;
    el.scrollLeft = Math.max(0, a.wx * zoom - a.vx);
    el.scrollTop = Math.max(0, a.wy * zoom - a.vy);
  });

  // 每個專案打開時自動全部顯示一次。分頁隱藏時畫面沒有尺寸，等看得到再做。
  useEffect(() => {
    if (!projectId || !isActive || agents.length === 0 || fittedFor.current === projectId) return;
    const h = window.requestAnimationFrame(() => {
      if (fitToView()) fittedFor.current = projectId;
    });
    return () => window.cancelAnimationFrame(h);
  }, [projectId, isActive, agents.length, fitToView]);

  // Ctrl／Cmd＋滾輪（含觸控板捏合）縮放。React 的 onWheel 是 passive，擋不掉頁面縮放，所以自己掛。
  useEffect(() => {
    const el = canvasRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      const r = el.getBoundingClientRect();
      zoomAround(zoomRef.current * Math.exp(-e.deltaY * 0.01), e.clientX - r.left, e.clientY - r.top);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [zoomAround, projects]);

  const world = useMemo(() => {
    let w = 0;
    let h = 0;
    for (const a of shownAgents) {
      w = Math.max(w, a.canvas_x + NODE_W);
      h = Math.max(h, a.canvas_y + NODE_H);
    }
    return { w: w + WORLD_MARGIN, h: h + WORLD_MARGIN };
  }, [shownAgents]);

  const summary = useMemo(() => projectSummary(agents, tasks), [agents, tasks]);
  const hasOrchestrator = agents.some((a) => a.kind === "orchestrator");

  // ── 活動記錄拖曳調整高度 ──────────────────────────────────────────────────
  const beginLogResize = (e: ReactMouseEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const startY = e.clientY;
    const startH = layout.logHeight;
    const max = Math.max(80, Math.round(window.innerHeight * 0.6));
    let h = startH;
    const onMove = (ev: MouseEvent) => {
      h = Math.min(max, Math.max(60, startH + (startY - ev.clientY)));
      setLayoutState((cur) => ({ ...cur, logHeight: h }));
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      setLayout({ logHeight: h });
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  const summaryLine = (sm: ProjectSummary): { text: string; tone: "ok" | "bad" | "muted" } | null => {
    switch (sm.kind) {
      case "none":
        return null;
      case "working":
        return {
          text: t.ac_sum_working(sm.passed, sm.total) + (sm.attention ? ` · ${t.ac_attention_count(sm.attention)}` : ""),
          tone: sm.attention ? "bad" : "muted",
        };
      case "all_passed":
        return { text: `✓ ${t.ac_sum_all_passed(sm.total)}`, tone: "ok" };
      case "merging":
        return { text: t.ac_sum_merging(sm.total), tone: "muted" };
      case "integrated":
        return { text: `✓ ${t.ac_sum_integrated(sm.total)}`, tone: "ok" };
      case "in_repo":
        return { text: `✓ ${t.ac_sum_in_repo(sm.total)}`, tone: "ok" };
      case "repo_pending": {
        const why = {
          repo_dirty: t.ac_repo_why_dirty,
          repo_merge_in_progress: t.ac_repo_why_in_progress,
          repo_diverged: t.ac_repo_why_diverged,
        }[sm.reason];
        return { text: t.ac_sum_repo_pending(sm.total, why), tone: "bad" };
      }
    }
  };
  // 在等你的 Agent：停下來等回覆的、與執行中的互動卡（要你手動完成）。
  const waitingYou = tasks.filter((c) => c.status === "running" && c.agent_id && (c.reply_wait_at || c.interactive)).length;
  const baseLine = summaryLine(summary);
  const sumLine = waitingYou
    ? { text: [baseLine?.text, t.ac_sum_waiting_you(waitingYou)].filter(Boolean).join(" · "), tone: "bad" as const }
    : baseLine;

  // ── 畫面 ───────────────────────────────────────────────────────────────

  if (projects !== null && projects.length === 0) {
    return (
      <div className="task-board agent-canvas">
        <div className="ac-empty-state">{t.ac_no_projects}</div>
      </div>
    );
  }

  return (
    <div
      className="task-board agent-canvas"
      style={{ gridTemplateRows: `auto auto minmax(0, 1fr) ${layout.logCollapsed ? "auto" : `${layout.logHeight}px`}` }}
    >
      <div className="ac-top">
        <h1 className="ac-title">
          <WorkflowIcon size={16} className="ac-title-icon" aria-hidden="true" />
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
        {sumLine && (
          <span className={`ac-summary ac-summary--${sumLine.tone}`} data-testid="ac-summary">
            {sumLine.text}
          </span>
        )}
        {projectUsage?.cards ? (
          <span className="ac-tokens" data-testid="ac-tokens" title={t.ac_tokens_tip(usageBreakdown(projectUsage))}>
            {t.ac_tokens(formatTokens(totalTokens(projectUsage.usage)))}
          </span>
        ) : null}
        {summary.kind === "in_repo" && (
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            onClick={() => void openRepo(summary.repoDir).catch((e) => setToast(errText(e)))}
          >
            {t.ac_open_folder}
          </button>
        )}
        <span className="ac-spacer" />
        <button
          className={`aiterm-btn aiterm-btn--sm ${questions.length ? "aiterm-btn--primary" : "aiterm-btn--ghost"}`}
          data-testid="ac-inbox-button"
          onClick={() => setInbox({ open: !inbox.open, focus: null })}
        >
          {t.ac_inbox_button(questions.length)}
        </button>
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

      <div className={`ac-main${layout.libCollapsed ? " ac-main--lib-collapsed" : ""}`}>
        {layout.libCollapsed ? (
          <aside className={`ac-lib ac-lib--rail${hover?.kind === "library" ? " ac-lib--drop" : ""}`} data-drop="library">
            <button
              className="ac-iconbtn ac-lib-toggle"
              title={t.ac_lib_expand}
              aria-label={t.ac_lib_expand}
              onClick={() => setLayout({ libCollapsed: false })}
            >
              »
            </button>
            <span className="ac-lib-rail-label">
              {t.ac_library}
              {library.length > 0 && ` · ${library.length}`}
            </span>
          </aside>
        ) : (
          <aside className={`ac-lib${hover?.kind === "library" ? " ac-lib--drop" : ""}`} data-drop="library">
            <div className="ac-lib-head">
              <h2>{t.ac_library}</h2>
              <button
                className="ac-iconbtn ac-lib-toggle"
                title={t.ac_lib_collapse}
                aria-label={t.ac_lib_collapse}
                onClick={() => setLayout({ libCollapsed: true })}
              >
                «
              </button>
            </div>
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
        )}

        <div className="ac-canvas-wrap">
          <section className={`ac-canvas${hover?.kind === "canvas" && ghost?.kind === "pool" ? " ac-canvas--drop" : ""}`} data-drop="canvas" ref={canvasRef}>
            {agents.length === 0 && <div className="ac-canvas-empty">{t.ac_empty_canvas}</div>}
            <div className="ac-sizer" style={{ width: world.w * zoom, height: world.h * zoom }}>
              <div
                className="ac-world"
                data-testid="ac-world"
                style={{ width: world.w, height: world.h, transform: `scale(${zoom})` }}
              >
                <EdgeLayer
                  containerRef={canvasRef}
                  agents={shownAgents}
                  edges={edges}
                  active={active}
                  pulses={pulses}
                  revision={tasks}
                  dispatchHint={t.ac_dispatch_edge_hint}
                />
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
                      hasOrchestrator={hasOrchestrator}
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
                      usage={usage}
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
                onMarkDone={(taskId) => void markDone(taskId)}
                question={questions.find((q) => q.project_id === projectId && q.agent_id === a.id) ?? null}
                onOpenQuestion={(id) => setInbox({ open: true, focus: id })}
                    />
                  );
                })}
              </div>
            </div>
          </section>
          {agents.length > 0 && (
            <div className="ac-zoom" role="toolbar">
              <button className="ac-iconbtn" title={t.ac_zoom_out} aria-label={t.ac_zoom_out} onClick={() => zoomAround(zoom / 1.2)}>
                −
              </button>
              <span className="ac-zoom-pct" data-testid="ac-zoom-pct">
                {Math.round(zoom * 100)}%
              </span>
              <button className="ac-iconbtn" title={t.ac_zoom_in} aria-label={t.ac_zoom_in} onClick={() => zoomAround(zoom * 1.2)}>
                +
              </button>
              <button className="ac-iconbtn ac-zoom-fit" onClick={() => fitToView()}>
                {t.ac_zoom_fit}
              </button>
            </div>
          )}
        </div>
      </div>

      <div className={`ac-log${layout.logCollapsed ? " ac-log--collapsed" : ""}`}>
        {!layout.logCollapsed && (
          <div
            className="ac-log-resize"
            title={t.ac_log_resize}
            aria-label={t.ac_log_resize}
            role="separator"
            aria-orientation="horizontal"
            onMouseDown={beginLogResize}
          />
        )}
        <button
          className="ac-log-title"
          aria-expanded={!layout.logCollapsed}
          title={layout.logCollapsed ? t.ac_log_expand : t.ac_log_collapse}
          onClick={() => setLayout({ logCollapsed: !layout.logCollapsed })}
        >
          {layout.logCollapsed ? "▸" : "▾"} {t.ac_log_title}
          {layout.logCollapsed && activity[0] && <span className="ac-log-latest">{activity[0].text}</span>}
        </button>
        {!layout.logCollapsed && (
          <div className="ac-log-lines">
            {activity.length === 0 && <div className="ac-log-empty">{t.ac_log_empty}</div>}
            {activity.map((ev, i) => (
              <div key={i} className={`ac-log-line ac-log-line--${ev.kind}`}>
                <span className="ac-log-time">{new Date(ev.at * 1000).toLocaleTimeString()}</span>
                {ev.text}
              </div>
            ))}
          </div>
        )}
      </div>

      {inbox.open && (
        <QuestionInbox
          questions={questions}
          focusId={inbox.focus}
          onAnswer={doAnswer}
          onClose={() => setInbox({ open: false, focus: null })}
        />
      )}

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
