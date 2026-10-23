import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { nanoid } from "nanoid";
import { useLocale } from "../../contexts/LocaleContext";
import { archExportText, archLoad, archQaLoad, archQaSave, archReadFiles, archSave, archScan, type ScanResult } from "../../ipc/architecture";
import { abortAi, formatAiError, invokeAiComplete, type AiError } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { ModelPickerButton } from "../ModelPickerButton";
import { ANALYZE_SYSTEM, buildAnalyzeMessage, mergeScanEdges, parseAnalysis, pickKeyFiles } from "./analyze";
import { buildBaseGraph, type Architecture, type Edge } from "./model";
import { buildDrillGraph, drillKey } from "./drill";
import { edgeLength, edgeMidpoint, edgePath, initialView, isCompact, layerColor, layoutGraph, neighborhood, spreadLabels, trueFitView, zoomAt, MAX_ZOOM, MIN_ZOOM, ZOOM_STEP, type Layout, type View } from "./layout";
import { DetailPanel } from "./DetailPanel";
import { SidePane } from "./SidePane";
import { useSidePane } from "./useSidePane";
import { FilePreview } from "./FilePreview";
import { QaPanel, type QaMessage, type QaStage } from "./QaPanel";
import { TourBar } from "./TourBar";
import { ANSWER_SYSTEM, SELECT_SYSTEM, attachEvidenceText, buildAnswerMessage, buildSelectMessage, parseAnswer, parseSelect, type Tour } from "./qa";
import { clampStep, tourRoute } from "./tour";
import { parseStoredQa, qaToMarkdown, serializeQa } from "./qaStore";
import "./index.css";

const PROVIDER_KEY = "aiterm_arch_provider";
const ROOT_KEY = "aiterm_arch_root";
/** 問答每個檔案最多讀這麼多位元組（分析只讀 6000）：控制器的後半段方法才不會被截掉、AI 才看得到回傳那一行。 */
const QA_FILE_BYTES = 12000;

/** 鑽取路徑上的一層：模組 id、它的檔案、顯示名稱（麵包屑用）。 */
interface DrillLevel { id: string; files: string[]; name: string }

/** 只含指定檔案、以及兩端都在其中的 import 的掃描結果，讓 AI 細分只看這個模組。 */
function subScanOf(scan: ScanResult, files: string[]): ScanResult {
  const own = new Set(files);
  return {
    ...scan,
    files: scan.files.filter((f) => own.has(f.path)),
    edges: scan.edges.filter((e) => own.has(e.from) && own.has(e.to)),
  };
}

/** 根層重新分析後，只保留仍存在的模組的 drills（鍵的第一段是根層模組 id）。 */
function pruneDrills(drills: Architecture["drills"], moduleIds: Set<string>): Architecture["drills"] {
  if (!drills) return undefined;
  const kept = Object.fromEntries(Object.entries(drills).filter(([k]) => moduleIds.has(k.split("\u0000")[0])));
  return Object.keys(kept).length > 0 ? kept : undefined;
}

/** 使用者要求減少動態時不渲染光點。jsdom 等環境沒有 matchMedia，查不到就當作沒開。 */
function prefersReducedMotion(): boolean {
  try { return !!window.matchMedia?.("(prefers-reduced-motion: reduce)")?.matches; } catch { return false; }
}

function samePath(a: Edge[] | null, b: Edge[]): boolean {
  return !!a && a.length === b.length && a.every((e, i) => e === b[i]);
}

/** 標籤互相錯開的最小垂直距離（標籤高約 18px）。 */
const LABEL_GAP = 20;
const DOTS_PER_EDGE = 2;
/** 連線多於這個數就當作稠密圖：沒有焦點時連線畫成安靜的底圖。 */
const DENSE_EDGES = 40;

type Status = { kind: "idle" } | { kind: "busy"; msg: string } | { kind: "error"; msg: string } | { kind: "notice"; msg: string };

/** Tauri IPC 的錯誤常是字串或物件，String(e) 對物件只會得到 [object Object]。 */
function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object") {
    const m = (e as { message?: unknown }).message;
    if (typeof m === "string") return m;
    try { return JSON.stringify(e); } catch { /* fall through */ }
  }
  return String(e);
}

export function ArchitectureView({ isActive }: { isActive: boolean }) {
  const { t } = useLocale();
  const [root, setRoot] = useState<string | null>(null);
  const [scan, setScan] = useState<ScanResult | null>(null);
  const [arch, setArch] = useState<Architecture | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const side = useSidePane();
  const [drillPath, setDrillPath] = useState<DrillLevel[]>([]);
  const [stale, setStale] = useState(false);
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [analyzing, setAnalyzing] = useState(false);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState("");
  const [elapsed, setElapsed] = useState(0);
  const [hoverEdge, setHoverEdge] = useState<number | null>(null);
  const [hoverNode, setHoverNode] = useState<string | null>(null);
  const [viewport, setViewport] = useState({ w: 0, h: 0 });
  // 使用者手動縮放／平移的結果；綁定產生它的 layout，換圖（鑽取、重新分析）就自動回到適合視窗。
  const [manual, setManual] = useState<{ layout: Layout; view: View } | null>(null);
  // 目前選的資料流路徑；綁定當時的圖與選取，換選取或換層級就自動失效。
  const [pathPick, setPathPick] = useState<{ path: Edge[]; view: Architecture; selected: string } | null>(null);
  const [panning, setPanning] = useState(false);
  const [preview, setPreview] = useState<{ root: string; path: string; line?: number } | null>(null);
  const openFile = useCallback((path: string, line?: number) => root && setPreview({ root, path, line }), [root]);
  const closePreview = useCallback(() => setPreview(null), []);
  const streamIdRef = useRef<string | null>(null);
  const runRef = useRef(0);
  const [qaOpen, setQaOpen] = useState(false);
  const [qaMessages, setQaMessages] = useState<QaMessage[]>([]);
  const [qaBusy, setQaBusy] = useState(false);
  const [qaStage, setQaStage] = useState<QaStage>("select");
  const [qaElapsed, setQaElapsed] = useState(0);
  const [tour, setTour] = useState<{ tour: Tour; index: number; playing: boolean } | null>(null);
  /** 刪除單題或清除記錄前的完整記錄；「復原刪除」用。開新資料夾、再問新題時作廢。 */
  const [qaUndo, setQaUndo] = useState<QaMessage[] | null>(null);
  const qaRunRef = useRef(0);
  /** 問答記錄已從哪個資料夾讀回；與目前 root 相同才會存檔，避免讀回之前就用空陣列蓋掉舊記錄。 */
  const qaLoadedRef = useRef<string | null>(null);
  /** 最後一次讀回或寫出的內容；沒變就不寫檔（開啟資料夾不該憑空產生空記錄檔）。 */
  const qaSavedRef = useRef("");
  const qaStreamRef = useRef<string | null>(null);
  const canvasRef = useRef<HTMLDivElement>(null);
  const archRef = useRef<Architecture | null>(null);
  const viewRef = useRef<View>({ scale: 1, tx: 0, ty: 0 });
  const layoutRef = useRef<Layout | null>(null);
  const dragCleanupRef = useRef<(() => void) | null>(null);

  /** 載入成功回 true。restoring＝自動還原上次的資料夾：失敗（多半是資料夾不見了）就靜靜回到空狀態。 */
  const load = useCallback(async (dir: string, restoring = false): Promise<boolean> => {
    setRoot(dir);
    setSelected(null);
    setDrillPath([]);
    setTour(null);
    setQaMessages([]);
    setQaUndo(null);
    qaLoadedRef.current = null;
    qaRunRef.current++;
    setQaBusy(false);
    setStatus({ kind: "busy", msg: t.arch_scanning });
    try {
      const s = await archScan(dir);
      setScan(s);
      const cached = await archLoad(dir);
      if (cached) {
        const parsed = JSON.parse(cached) as Architecture;
        setArch(parsed);
        setStale(parsed.meta.gitHead !== s.git_head);
      } else {
        setArch(buildBaseGraph(s));
        setStale(false);
      }
      try {
        const stored = await archQaLoad(dir);
        const restored = stored ? parseStoredQa(stored) : [];
        setQaMessages(restored);
        qaSavedRef.current = serializeQa(restored);
        qaLoadedRef.current = dir;
      } catch { /* 讀不到記錄就當作沒有；qaLoadedRef 維持 null，這次不會寫檔 */ }
      setStatus(s.truncated ? { kind: "notice", msg: t.arch_truncated_n.replace("{n}", String(s.files.length)) } : { kind: "idle" });
      return true;
    } catch (e) {
      if (restoring) {
        try { localStorage.removeItem(ROOT_KEY); } catch { /* 清不掉也只是下次再試一次 */ }
        setRoot(null);
        setScan(null);
        setArch(null);
        setStatus({ kind: "idle" });
      } else {
        setStatus({ kind: "error", msg: t.arch_error.replace("{msg}", errText(e)) });
      }
      return false;
    }
  }, [t]);

  const pick = useCallback(async () => {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir !== "string") return;
    if (await load(dir)) {
      try { localStorage.setItem(ROOT_KEY, dir); } catch { /* 記不住不影響使用 */ }
    }
  }, [load]);

  // 掛載時自動載入上次的資料夾（只做一次，StrictMode 的重複掛載也不重掃）。
  const restoredRef = useRef(false);
  useEffect(() => {
    if (restoredRef.current) return;
    restoredRef.current = true;
    let saved: string | null = null;
    try { saved = localStorage.getItem(ROOT_KEY); } catch { /* 讀不到就當沒有 */ }
    if (saved) void load(saved, true);
  }, [load]);

  // 可選的模型：上次記住的 → 預設供應商 → 清單第一個。記住的 id 若已被刪除要退回預設。
  useEffect(() => {
    let alive = true;
    void listProviders().then((list) => {
      if (!alive) return;
      setProviders(list);
      let remembered: string | null = null;
      try { remembered = localStorage.getItem(PROVIDER_KEY); } catch { /* 沒有儲存空間就用預設 */ }
      const fallback = list.find((p) => p.is_default)?.id ?? list[0]?.id ?? "";
      setProviderId(remembered && list.some((p) => p.id === remembered) ? remembered : fallback);
    }).catch(() => { /* 取不到清單就維持預設供應商 */ });
    return () => { alive = false; };
  }, []);

  const changeProvider = useCallback((id: string) => {
    setProviderId(id);
    try { localStorage.setItem(PROVIDER_KEY, id); } catch { /* 記不住不影響使用 */ }
  }, []);

  /**
   * 根層分析與模組細分共用：選供應商、已等時間、取消、錯誤處理都在這裡。
   * target 是要送給 AI 的掃描範圍；apply 把驗證過的結果併進目前的整份架構並回傳新的整份，
   * 這裡負責顯示與存檔。
   */
  const runAnalysis = useCallback(async (target: ScanResult, apply: (merged: Architecture, current: Architecture | null) => Architecture) => {
    if (!root || !scan) return;
    const run = ++runRef.current;
    const streamId = nanoid();
    streamIdRef.current = streamId;
    setElapsed(0);
    setAnalyzing(true);
    setStatus({ kind: "idle" });
    try {
      const base = buildBaseGraph(target);
      const files = await archReadFiles(root, pickKeyFiles(target));
      const reply = await invokeAiComplete(
        [{ role: "user", content: buildAnalyzeMessage(target, base, files) }],
        ANALYZE_SYSTEM,
        streamId,
        providerId || undefined,
      );
      if (run !== runRef.current) return; // 已取消
      const parsed = reply.content ? parseAnalysis(reply.content, target) : null;
      if (!parsed) {
        setStatus({ kind: "error", msg: t.arch_error.replace("{msg}", "AI 回應不是有效的架構 JSON") });
        return;
      }
      const next = apply(mergeScanEdges(parsed, target), archRef.current);
      setArch(next);
      setStale(next.meta.gitHead !== scan.git_head);
      await archSave(root, JSON.stringify(next));
      setStatus(parsed.meta.dropped > 0 ? { kind: "notice", msg: t.arch_dropped.replace("{n}", String(parsed.meta.dropped)) } : { kind: "idle" });
    } catch (e) {
      if (run !== runRef.current) return; // 取消造成的錯誤不顯示
      const err = e as AiError;
      setStatus(err?.kind === "not_configured"
        ? { kind: "notice", msg: t.arch_no_ai }
        : { kind: "error", msg: t.arch_error.replace("{msg}", err?.kind ? formatAiError(err) : errText(e)) });
    } finally {
      if (run === runRef.current) setAnalyzing(false);
    }
  }, [root, scan, t, providerId]);

  const analyze = useCallback(() => {
    if (!scan) return Promise.resolve();
    return runAnalysis(scan, (merged, current) => {
      // 根圖被取代：舊 drills 只留模組 id 仍存在的；鑽取位置可能失效，回到根層。
      setDrillPath([]);
      setSelected(null);
      setTour(null);
      const drills = pruneDrills(current?.drills, new Set(merged.modules.map((m) => m.id)));
      return drills ? { ...merged, drills } : merged;
    });
  }, [scan, runAnalysis]);

  const analyzeDrill = useCallback(() => {
    const level = drillPath[drillPath.length - 1];
    if (!scan || !level) return Promise.resolve();
    const key = drillKey(drillPath.map((d) => d.id));
    return runAnalysis(subScanOf(scan, level.files), (merged, current) => {
      if (!current) return merged;
      return { ...current, drills: { ...current.drills, [key]: merged } };
    });
  }, [scan, drillPath, runAnalysis]);

  const cancelAnalyze = useCallback(() => {
    runRef.current++;
    setAnalyzing(false);
    setStatus({ kind: "idle" });
    const id = streamIdRef.current;
    if (id) abortAi(id).catch(() => { /* 取消本身失敗也不打擾使用者 */ });
  }, []);

  /**
   * 架構問答：兩段式。(1) 只用架構摘要請 AI 挑檔；(2) 讀那些檔案再回答。
   * 每個問題各自獨立——刻意不把先前的對話歷史送給 AI（摘要＋檔案內容已是完整脈絡，歷史只會讓提示越來越長）。
   * 摘要一律用根層架構，即使目前鑽進了某個模組。取消用 run 計數器讓遲到的回覆被忽略。
   */
  const ask = useCallback(async (question: string) => {
    if (!arch || !scan || !root) return;
    const run = ++qaRunRef.current;
    setQaUndo(null);
    setQaMessages((m) => [...m, { id: nanoid(), role: "user", text: question, at: Date.now() }]);
    setQaElapsed(0);
    setQaStage("select");
    setQaBusy(true);
    setStatus((s) => (s.kind === "notice" || s.kind === "error" ? { kind: "idle" } : s));
    try {
      const id1 = nanoid();
      qaStreamRef.current = id1;
      const r1 = await invokeAiComplete([{ role: "user", content: buildSelectMessage(arch, question) }], SELECT_SYSTEM, id1, providerId || undefined);
      if (run !== qaRunRef.current) return;
      // 挑檔失敗或沒挑到檔案也繼續：只靠架構摘要回答。
      const picked = r1.content ? parseSelect(r1.content, arch, scan) : { modules: [], files: [] };
      setQaStage("answer");
      const files = picked.files.length > 0 ? await archReadFiles(root, picked.files, QA_FILE_BYTES) : [];
      if (run !== qaRunRef.current) return;
      const id2 = nanoid();
      qaStreamRef.current = id2;
      const r2 = await invokeAiComplete([{ role: "user", content: buildAnswerMessage(arch, question, files) }], ANSWER_SYSTEM, id2, providerId || undefined);
      if (run !== qaRunRef.current) return;
      if (!r2.content) {
        // 供應商沒給文字（例如只回了工具呼叫）：和「格式不對」分開講，也沒有原文可看。
        setQaMessages((m) => [...m, { id: nanoid(), role: "error", text: t.arch_qa_no_text }]);
      } else {
        const content = r2.content;
        const parsed = parseAnswer(content, arch, scan);
        const qa = parsed ? attachEvidenceText(parsed, files) : null;
        setQaMessages((m) => [...m, qa ? { id: nanoid(), role: "ai", qa } : { id: nanoid(), role: "error", text: t.arch_qa_bad_format, raw: content }]);
      }
    } catch (e) {
      if (run !== qaRunRef.current) return;
      const err = e as AiError;
      if (err?.kind === "not_configured") setStatus({ kind: "notice", msg: t.arch_no_ai });
      else setQaMessages((m) => [...m, { id: nanoid(), role: "error", text: t.arch_qa_error.replace("{msg}", err?.kind ? formatAiError(err) : errText(e)) }]);
    } finally {
      if (run === qaRunRef.current) setQaBusy(false);
    }
  }, [arch, scan, root, providerId, t]);

  const cancelAsk = useCallback(() => {
    qaRunRef.current++;
    setQaBusy(false);
    const id = qaStreamRef.current;
    if (id) abortAi(id).catch(() => { /* 取消本身失敗也不打擾使用者 */ });
  }, []);

  useEffect(() => {
    if (!qaBusy) return;
    const timer = setInterval(() => setQaElapsed((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, [qaBusy]);

  useEffect(() => {
    if (!root || qaBusy || qaLoadedRef.current !== root) return;
    const json = serializeQa(qaMessages);
    if (json === qaSavedRef.current) return;
    qaSavedRef.current = json;
    archQaSave(root, json).catch(() => { /* 存不下來不影響問答本身 */ });
  }, [root, qaBusy, qaMessages]);

  const exportQa = useCallback(async () => {
    if (!root) return;
    const name = root.split(/[\\/]/).filter(Boolean).pop() ?? "project";
    const d = new Date();
    const p2 = (n: number) => String(n).padStart(2, "0");
    try {
      const path = await save({
        defaultPath: `${name}-${t.arch_qa_md_title}-${d.getFullYear()}${p2(d.getMonth() + 1)}${p2(d.getDate())}.md`,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (!path) return;
      const names = new Map((arch?.modules ?? []).map((m) => [m.id, m.name]));
      const md = qaToMarkdown(qaMessages, name, d, {
        title: t.arch_qa_md_title, exportedAt: t.arch_qa_md_exported_at, question: t.arch_qa_md_question,
        tour: t.arch_qa_md_tour, diagram: t.arch_qa_md_diagram, truncated: t.arch_qa_md_truncated,
      }, (id) => names.get(id) ?? id);
      await archExportText(path, md);
      setStatus({ kind: "notice", msg: t.arch_qa_exported.replace("{path}", path) });
    } catch (e) {
      setStatus({ kind: "error", msg: t.arch_qa_export_failed.replace("{msg}", errText(e)) });
    }
  }, [root, arch, qaMessages, t]);

  const clearQa = useCallback(() => {
    setQaUndo(qaMessages.length > 0 ? qaMessages : null);
    setQaMessages([]);
    setTour(null);
  }, [qaMessages]);

  // 刪一題：連同它後面的回答（到下一個提問之前）一起刪。
  const deleteQuestion = useCallback((questionId: string) => {
    const from = qaMessages.findIndex((m) => m.id === questionId && m.role === "user");
    if (from < 0) return;
    let to = from + 1;
    while (to < qaMessages.length && qaMessages[to].role !== "user") to++;
    setQaUndo(qaMessages);
    setQaMessages([...qaMessages.slice(0, from), ...qaMessages.slice(to)]);
  }, [qaMessages]);
  const undoQa = useCallback(() => {
    if (!qaUndo) return;
    setQaMessages(qaUndo);
    setQaUndo(null);
  }, [qaUndo]);

  const playTour = useCallback((messageId: string) => {
    const msg = qaMessages.find((m) => m.id === messageId);
    if (!msg || msg.role !== "ai" || !msg.qa.tour) return;
    // 記錄是舊的：重新分析後模組可能不見了，只播放現在還存在的步驟。
    const alive = new Set((archRef.current?.modules ?? []).map((m) => m.id));
    const steps = msg.qa.tour.steps.filter((s) => alive.has(s.module));
    if (steps.length === 0) return;
    // 導覽在根層播放：離開鑽取、清掉選取與資料流路徑。
    setDrillPath([]);
    setSelected(null);
    setHoverEdge(null);
    setPathPick(null);
    setTour({ tour: { ...msg.qa.tour, steps }, index: 0, playing: !prefersReducedMotion() });
  }, [qaMessages]);
  const endTour = useCallback(() => setTour(null), []);
  const setTourIndex = useCallback((i: number) => setTour((c) => (c ? { ...c, index: clampStep(c.tour.steps, i) } : c)), []);
  const setTourPlaying = useCallback((p: boolean) => setTour((c) => (c ? { ...c, playing: p } : c)), []);

  useEffect(() => {
    if (!analyzing) return;
    const timer = setInterval(() => setElapsed((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, [analyzing]);

  useEffect(() => {
    const el = canvasRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      setViewport({ w: r?.width ?? 0, h: r?.height ?? 0 });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, [arch]);

  useEffect(() => { archRef.current = arch; }, [arch]);

  const drillKeyNow = drillKey(drillPath.map((d) => d.id));
  // 目前顯示的圖：根圖，或鑽取層（AI 細分結果優先，否則由掃描即時產生）。
  const drilledFiles = drillPath.length > 0 ? drillPath[drillPath.length - 1].files : null;
  const savedDrill = arch?.drills?.[drillKeyNow];
  const built = useMemo(() => (drilledFiles && scan ? buildDrillGraph(scan, drilledFiles) : null), [scan, drilledFiles]);
  const view = !arch ? null : !drilledFiles ? arch : savedDrill ?? built?.arch ?? null;
  const hiddenCount = drilledFiles && !savedDrill ? built?.hidden ?? 0 : 0;
  const hasAiDrill = !!savedDrill;

  const goTo = useCallback((depth: number) => {
    setDrillPath((p) => p.slice(0, depth));
    setSelected(null);
    setHoverEdge(null);
  }, []);
  const enter = useCallback((id: string) => {
    const m = view?.modules.find((x) => x.id === id);
    if (!m || m.files.length <= 1) return; // 單一檔案不能再鑽
    setDrillPath((p) => [...p, { id: m.id, files: m.files, name: m.name }]);
    setSelected(null);
    setHoverEdge(null);
  }, [view]);

  const layout = useMemo(() => (view ? layoutGraph(view) : null), [view]);
  // 焦點：選取的模組；沒選時滑過的模組（只預覽，不選取）。只強調一跳，整條鏈要靠資料流路徑。
  const focusId = selected ?? hoverNode;
  const near = useMemo(() => (view && focusId ? neighborhood(view, focusId) : null), [view, focusId]);
  const denseGraph = (view?.edges.length ?? 0) > DENSE_EDGES;

  const fit: View = layout
    ? initialView(layout.width, layout.height, viewport.w, viewport.h)
    : { scale: 1, tx: 0, ty: 0 };
  const cur: View = manual && manual.layout === layout ? manual.view : fit;
  const activePath = pathPick && pathPick.view === view && pathPick.selected === selected ? pathPick.path : null;
  const hl = useMemo(() => {
    // 導覽中：只亮目前這步與下一步的模組，以及兩者之間的連線；其餘變暗（優先於選取／路徑）。
    if (tour && arch && view === arch) {
      const { steps } = tour.tour;
      const modules = new Set<string>();
      for (const s of [steps[tour.index], steps[tour.index + 1]]) if (s) modules.add(s.module);
      // 兩步之間沒有直接連線時，沿途經過的模組也一起亮，才看得出資料走過哪幾層。
      const route = tourRoute(arch, steps, tour.index);
      for (const m of route.modules) modules.add(m);
      // 連續兩步在同一個模組裡（例如分派器→流程服務）沒有連線可畫：改在卡片旁畫一圈迴圈箭頭，並標出這一步在模組內從哪個檔案到哪個檔案。
      const cur = steps[tour.index];
      const nxt = steps[tour.index + 1];
      const base = (f?: string) => f?.split(/[\\/]/).pop();
      const a = base(cur?.evidence?.file);
      const b = base(nxt?.evidence?.file);
      const loop = cur && nxt && cur.module === nxt.module ? { module: cur.module, label: a && b && a !== b ? `${a} → ${b}` : t.arch_tour_inside } : null;
      return { edges: new Set<Edge>(route.edges), modules, reversed: route.reversed, loop, current: cur?.module };
    }
    if (activePath) {
      const modules = new Set<string>();
      for (const e of activePath) { modules.add(e.from); modules.add(e.to); }
      return { edges: new Set<Edge>(activePath), modules, reversed: undefined, loop: null, current: undefined };
    }
    return near ? { edges: new Set<Edge>(near.edges), modules: near.modules, reversed: undefined, loop: null, current: undefined } : null;
  }, [tour, arch, view, activePath, near, t]);
  const reduced = prefersReducedMotion();
  const pickPath = useCallback((p: Edge[]) => {
    if (!view || !selected) return;
    setPathPick((cur0) => (cur0 && cur0.view === view && cur0.selected === selected && samePath(cur0.path, p) ? null : { path: p, view, selected }));
  }, [view, selected]);
  const fitView = useCallback(() => {
    if (layout) setManual({ layout, view: trueFitView(layout.width, layout.height, viewport.w, viewport.h) });
  }, [layout, viewport]);
  const viewportRef = useRef(viewport);
  useEffect(() => { viewRef.current = cur; layoutRef.current = layout; viewportRef.current = viewport; });
  // 工具列的縮放：繞可視區中心（含內部捲動量，與滾輪一致）。
  const zoomCenter = useCallback((factor: number) => {
    const layoutNow = layoutRef.current;
    const el = canvasRef.current;
    if (!layoutNow || !el) return;
    const vp = viewportRef.current;
    setManual({ layout: layoutNow, view: zoomAt(viewRef.current, factor, el.scrollLeft + vp.w / 2, el.scrollTop + vp.h / 2) });
  }, []);
  const zoomIn = useCallback(() => zoomCenter(ZOOM_STEP), [zoomCenter]);
  const zoomOut = useCallback(() => zoomCenter(1 / ZOOM_STEP), [zoomCenter]);
  const zoomReset = useCallback(() => zoomCenter(1 / viewRef.current.scale), [zoomCenter]);
  useEffect(() => () => dragCleanupRef.current?.(), []);

  // 導覽換步時把目前模組的卡片捲進可視範圍（jsdom 沒有 scrollIntoView）。
  const tourModule = tour ? tour.tour.steps[tour.index]?.module : undefined;
  // 檔案預覽蓋住畫布右側時，把導覽（或選取）的模組平移到預覽左邊的可見區。
  const previewOpen = !!(root && preview && preview.root === root);
  // 預覽關掉時，若畫布還停在我們平移過去的位置（使用者沒再動它），推回原位。
  const panBackRef = useRef<{ layout: Layout; from: number; to: number } | null>(null);
  useEffect(() => {
    if (previewOpen) return;
    const back = panBackRef.current;
    panBackRef.current = null;
    if (back && layoutRef.current === back.layout && Math.abs(viewRef.current.tx - back.to) < 1) {
      setManual({ layout: back.layout, view: { ...viewRef.current, tx: back.from } });
    }
  }, [previewOpen]);
  const focusModules = tour && hl ? hl.modules : selected ? new Set([selected]) : null;
  useEffect(() => {
    const canvas = canvasRef.current;
    const lay = layoutRef.current;
    if (!previewOpen || !canvas || !lay || !focusModules) return;
    const pv = canvas.parentElement?.querySelector<HTMLElement>(".arch-preview")?.getBoundingClientRect();
    if (!pv || pv.width === 0) return;
    const cr = canvas.getBoundingClientRect();
    let left = Infinity;
    let right = -Infinity;
    for (const n of Array.from(canvas.querySelectorAll<HTMLElement>("[data-mid]"))) {
      if (!focusModules.has(n.dataset.mid ?? "")) continue;
      const r = n.getBoundingClientRect();
      left = Math.min(left, r.left);
      right = Math.max(right, r.right);
    }
    if (!isFinite(left)) return;
    const MARGIN = 16;
    const overflow = right - (pv.left - MARGIN);
    if (overflow <= 0) return;
    // 最多往左推到最左邊的卡片貼齊可視區左緣，不把它推出畫面。
    const delta = Math.min(overflow, Math.max(0, left - (cr.left + MARGIN)));
    if (delta < 1) return;
    // 連續幾步都在平移時，要推回的是最初那個位置。
    const prevPan = panBackRef.current;
    const origin = prevPan && prevPan.layout === lay && Math.abs(viewRef.current.tx - prevPan.to) < 1 ? prevPan.from : viewRef.current.tx;
    panBackRef.current = { layout: lay, from: origin, to: viewRef.current.tx - delta };
    setManual({ layout: lay, view: { ...viewRef.current, tx: viewRef.current.tx - delta } });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- focusModules 每次渲染都是新 Set，只在換步／開預覽／換選取時重算。
  }, [previewOpen, tourModule, selected]);

  useEffect(() => {
    if (!tourModule || !layout) return;
    const el = Array.from(canvasRef.current?.querySelectorAll<HTMLElement>("[data-mid]") ?? []).find((n) => n.dataset.mid === tourModule);
    el?.scrollIntoView?.({ block: "nearest", inline: "nearest", behavior: prefersReducedMotion() ? "auto" : "smooth" });
  }, [tourModule, layout]);

  // Ctrl/⌘＋滾輪以游標為中心縮放；一般滾輪維持捲動。要 preventDefault 就必須是非 passive 的原生監聽。
  useEffect(() => {
    const el = canvasRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      const layoutNow = layoutRef.current;
      if (!layoutNow) return;
      e.preventDefault();
      const r = el.getBoundingClientRect();
      const factor = Math.exp(-e.deltaY * 0.0015);
      setManual({ layout: layoutNow, view: zoomAt(viewRef.current, factor, e.clientX - r.left + el.scrollLeft, e.clientY - r.top + el.scrollTop) });
    };
    // macOS WKWebView 的觸控板捏合是 Safari 的 gesture* 事件（不是 ctrl+wheel）；scale 是自 gesturestart 起的累計倍率。
    let lastScale = 1;
    const onGestureStart = (e: Event) => { e.preventDefault(); lastScale = 1; };
    const onGestureChange = (e: Event) => {
      e.preventDefault();
      const layoutNow = layoutRef.current;
      const g = e as Event & { scale?: number; clientX?: number; clientY?: number };
      if (!layoutNow || !g.scale || !(g.scale > 0)) return;
      const r = el.getBoundingClientRect();
      const factor = g.scale / lastScale;
      lastScale = g.scale;
      setManual({ layout: layoutNow, view: zoomAt(viewRef.current, factor, (g.clientX ?? 0) - r.left + el.scrollLeft, (g.clientY ?? 0) - r.top + el.scrollTop) });
    };
    const onGestureEnd = (e: Event) => { e.preventDefault(); lastScale = 1; };
    el.addEventListener("wheel", onWheel, { passive: false });
    el.addEventListener("gesturestart", onGestureStart as EventListener);
    el.addEventListener("gesturechange", onGestureChange as EventListener);
    el.addEventListener("gestureend", onGestureEnd as EventListener);
    return () => {
      el.removeEventListener("wheel", onWheel);
      el.removeEventListener("gesturestart", onGestureStart as EventListener);
      el.removeEventListener("gesturechange", onGestureChange as EventListener);
      el.removeEventListener("gestureend", onGestureEnd as EventListener);
    };
  }, []);

  const startPan = (e: React.MouseEvent) => {
    if (e.button !== 0 || !layoutRef.current) return;
    if ((e.target as Element).closest(".arch-node, .arch-zoom, .arch-empty")) return;
    const layoutNow = layoutRef.current;
    const start = { x: e.clientX, y: e.clientY, view: viewRef.current };
    const move = (m: MouseEvent) => setManual({ layout: layoutNow, view: { ...start.view, tx: start.view.tx + m.clientX - start.x, ty: start.view.ty + m.clientY - start.y } });
    const stop = () => { window.removeEventListener("mousemove", move); window.removeEventListener("mouseup", stop); dragCleanupRef.current = null; setPanning(false); };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", stop);
    dragCleanupRef.current = stop;
    setPanning(true);
  };

  const labelPts = useMemo(() => {
    if (!view || !layout) return [];
    // 同一對模組、同一種連線（例如 9 條 import）合成一顆標籤，顯示條數。
    const groups = new Map<string, { id: string; x: number; y: number; count: number }>();
    view.edges.forEach((e, i) => {
      const a = layout.nodes[e.from];
      const b = layout.nodes[e.to];
      if (!a || !b || !(hl?.edges.has(e) || hoverEdge === i)) return;
      const key = `${e.from}>${e.to}>${e.kind}`;
      const g = groups.get(key);
      if (g) { g.count++; return; }
      const m = edgeMidpoint(a, b);
      groups.set(key, { id: String(i), x: m.x, y: m.y, count: 1 });
    });
    const list = [...groups.values()];
    const counts = new Map(list.map((g) => [g.id, g.count]));
    return spreadLabels(list, LABEL_GAP).map((p) => ({ ...p, count: counts.get(p.id) ?? 1 }));
  }, [view, layout, hl, hoverEdge]);
  const layerIndex = (id: string) => Math.max(0, view?.layers.findIndex((l) => l.id === id) ?? 0);
  const mmss = `${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, "0")}`;
  const pickIcon = (
    <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.4" aria-hidden="true">
      <rect x="3" y="4" width="7" height="6" rx="1.5" /><rect x="14" y="4" width="7" height="6" rx="1.5" /><rect x="8.5" y="14" width="7" height="6" rx="1.5" />
      <path d="M6.5 10v2a2 2 0 0 0 2 2M17.5 10v2a2 2 0 0 1-2 2" />
    </svg>
  );

  return (
    <div className="arch-view" style={{ display: isActive ? undefined : "none" }}>
      {root && (
        <header className="arch-toolbar">
          <button type="button" className="arch-btn" onClick={pick} disabled={analyzing}>{t.arch_pick_folder}</button>
          <span className="arch-root" title={root}>{root}</span>
          <button type="button" className="arch-btn" onClick={() => load(root)} disabled={analyzing}>{t.arch_rescan}</button>
          {scan && providers.length > 0 && (
            <ModelPickerButton providers={providers} selectedId={providerId} onChange={changeProvider} disabled={analyzing} />
          )}
          {scan && !analyzing && (
            <button type="button" className="arch-btn arch-btn-primary" onClick={analyze} disabled={status.kind === "busy"}>{t.arch_analyze}</button>
          )}
          {scan && arch && (
            <button type="button" className={`arch-btn ${qaOpen ? "arch-btn-active" : ""}`} aria-pressed={qaOpen} onClick={() => setQaOpen((o) => !o)}>{t.arch_qa_toggle}</button>
          )}
          {analyzing && (
            <span className="arch-analyzing" role="status">
              <i className="arch-spinner" />
              <span>{`${t.arch_analyzing} ${mmss}`}</span>
              <button type="button" className="arch-btn arch-btn-small" onClick={cancelAnalyze}>{t.arch_cancel}</button>
            </span>
          )}
          <span className="arch-legend"><i className="arch-line-solid" />{t.arch_legend_scan}<i className="arch-line-dash" />{t.arch_legend_ai}</span>
        </header>
      )}
      {drillPath.length > 0 && (
        <nav className="arch-crumbs" aria-label={t.arch_crumb_nav}>
          <button type="button" className="arch-crumb" onClick={() => goTo(0)}>{t.arch_crumb_root}</button>
          {drillPath.map((d, i) => (
            <span key={`${i}-${d.id}`} className="arch-crumb-item">
              <span className="arch-crumb-sep" aria-hidden="true">›</span>
              {i < drillPath.length - 1
                ? <button type="button" className="arch-crumb" onClick={() => goTo(i + 1)}>{d.name}</button>
                : <span className="arch-crumb-cur" aria-current="page">{d.name}</span>}
            </span>
          ))}
          {scan && !analyzing && (
            <button type="button" className="arch-btn arch-btn-small" onClick={analyzeDrill} disabled={status.kind === "busy"}>
              {hasAiDrill ? t.arch_drill_reanalyze : t.arch_drill_analyze}
            </button>
          )}
        </nav>
      )}
      {hiddenCount > 0 && <div className="arch-status arch-status-notice">{t.arch_hidden_n.replace("{n}", String(hiddenCount))}</div>}
      {status.kind !== "idle" && <div className={`arch-status arch-status-${status.kind}`}>{status.msg}</div>}
      {stale && <div className="arch-status arch-status-notice">{t.arch_stale}</div>}
      <div className="arch-body">
        <div className={`arch-canvas-wrap ${previewOpen ? "arch-has-preview" : ""}`}>
        <div className={`arch-canvas ${panning ? "arch-panning" : ""}`} ref={canvasRef} onMouseDown={startPan}>
          {!root && (
            <div className="arch-empty">
              <div className="arch-empty-card">
                <div className="arch-empty-icon">{pickIcon}</div>
                <h2>{t.arch_tab}</h2>
                <p>{t.new_arch_desc}</p>
                <button type="button" className="arch-btn arch-btn-primary arch-btn-big" onClick={pick}>{t.arch_pick_folder}</button>
              </div>
            </div>
          )}
          {view && layout && (
            <div className="arch-stage-wrap" style={{ width: layout.width * cur.scale + Math.max(0, cur.tx), height: layout.height * cur.scale + Math.max(0, cur.ty) }}>
              <div className="arch-stage" style={{ width: layout.width, height: layout.height, transform: `translate(${cur.tx}px, ${cur.ty}px) scale(${cur.scale})` }}>
                {layout.columns.map((c, i) => (
                  <div key={c.layerId} className="arch-lane" style={{ left: c.x, top: c.y, width: c.w, height: c.h, ["--lc" as string]: layerColor(c.layerId, i) }}>
                    <div className="arch-lane-head">
                      <i className="arch-dot" />
                      <span className="arch-lane-name" title={c.name}>{c.name}</span>
                      <span className="arch-lane-count">{c.count}</span>
                    </div>
                  </div>
                ))}
                <svg width={layout.width} height={layout.height} className="arch-edges">
                  <defs>
                    <marker id="arch-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M 0 1 L 9 5 L 0 9 z" className="arch-arrow" /></marker>
                    <marker id="arch-arrow-on" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M 0 1 L 9 5 L 0 9 z" className="arch-arrow arch-arrow-on" /></marker>
                    <filter id="arch-glow" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="4" /></filter>
                  </defs>
                  {view.edges.map((e, i) => {
                    const a = layout.nodes[e.from];
                    const b = layout.nodes[e.to];
                    if (!a || !b) return null;
                    const on = hl ? hl.edges.has(e) : false;
                    // 導覽中資料往回走（例如控制器回傳給前端）時，箭頭與光點倒過來，跟字幕說的方向一致。
                    const rev = on && !!hl?.reversed?.has(e);
                    const d = edgePath(a, b);
                    const dur = Math.min(4, Math.max(1, edgeLength(a, b) / 160));
                    const quiet = !hl && denseGraph;
                    return (
                      <g key={i} onMouseEnter={() => setHoverEdge(i)} onMouseLeave={() => setHoverEdge(null)}>
                        {on && <path d={d} fill="none" className="arch-edge-glow" filter="url(#arch-glow)" />}
                        <path d={d} fill="none" {...{ [rev ? "markerStart" : "markerEnd"]: `url(#${on ? "arch-arrow-on" : "arch-arrow"})` }} data-on={on ? "true" : "false"} data-rev={rev ? "true" : undefined}
                          className={`arch-edge ${e.source === "ai" ? "arch-edge-ai" : ""} ${on ? "arch-edge-on" : ""} ${hl && !on ? "arch-edge-dim" : ""} ${quiet ? "arch-edge-quiet" : ""}`}>
                          <title>{e.note ?? e.kind}</title>
                        </path>
                        <path d={d} fill="none" className="arch-edge-hit" />
                        {on && !reduced && Array.from({ length: DOTS_PER_EDGE }, (_, k) => (
                          <circle key={k} r="3.5" className="arch-flow-dot">
                            <animateMotion path={d} {...(rev ? { keyPoints: "1;0", keyTimes: "0;1", calcMode: "linear" } : {})} dur={`${dur.toFixed(2)}s`} begin={k === 0 ? "0s" : `-${((dur * k) / DOTS_PER_EDGE).toFixed(2)}s`} repeatCount="indefinite" />
                          </circle>
                        ))}
                      </g>
                    );
                  })}
                  {hl?.loop && layout.nodes[hl.loop.module] && (() => {
                    const n = layout.nodes[hl.loop.module];
                    const x = n.x + n.w;
                    const d = `M ${x} ${n.y + n.h * 0.3} C ${x + 46} ${n.y + n.h * 0.05}, ${x + 46} ${n.y + n.h * 0.95}, ${x} ${n.y + n.h * 0.7}`;
                    return (
                      <g data-testid="tour-loop">
                        <path d={d} fill="none" markerEnd="url(#arch-arrow-on)" className="arch-edge arch-edge-on" />
                        {!reduced && Array.from({ length: DOTS_PER_EDGE }, (_, k) => (
                          <circle key={k} r="3.5" className="arch-flow-dot">
                            <animateMotion path={d} dur="1.6s" begin={k === 0 ? "0s" : "-0.8s"} repeatCount="indefinite" />
                          </circle>
                        ))}
                      </g>
                    );
                  })()}
                </svg>
                {view.modules.map((m) => {
                  const p = layout.nodes[m.id];
                  if (!p) return null;
                  const dim = hl && !hl.modules.has(m.id);
                  const compact = isCompact(m);
                  return (
                    <button key={m.id} type="button" title={m.name} aria-label={m.name} data-mid={m.id}
                      className={`arch-node ${compact ? "arch-node-compact" : ""} ${selected === m.id ? "arch-node-sel" : ""} ${hl?.current === m.id ? "arch-node-tour" : ""} ${dim ? "arch-node-dim" : ""}`}
                      style={{ left: p.x, top: p.y, width: p.w, height: p.h, ["--lc" as string]: layerColor(m.layer, layerIndex(m.layer)) }}
                      onClick={() => { setSelected(m.id); side.expand(); }} onDoubleClick={() => enter(m.id)}
                      onMouseEnter={() => setHoverNode(m.id)} onMouseLeave={() => setHoverNode((h) => (h === m.id ? null : h))}>
                      <span className="arch-node-head">
                        <span className="arch-node-name">{m.name}</span>
                        <span className="arch-node-files">{t.arch_files_count.replace("{n}", String(m.files.length))}</span>
                      </span>
                      {!compact && m.summary && <span className="arch-node-sum">{m.summary}</span>}
                      {!compact && (
                        <span className="arch-node-foot">
                          {m.tech.slice(0, 2).map((x) => <span key={x} className="arch-chip arch-chip-sm">{x}</span>)}
                        </span>
                      )}
                    </button>
                  );
                })}
                {hl?.loop && layout.nodes[hl.loop.module] && (
                  <span className="arch-edge-pill arch-loop-pill" style={{ left: layout.nodes[hl.loop.module].x + layout.nodes[hl.loop.module].w / 2, top: layout.nodes[hl.loop.module].y + layout.nodes[hl.loop.module].h + 18, maxWidth: layout.nodes[hl.loop.module].w }} title={hl.loop.label}>{hl.loop.label}</span>
                )}
                {labelPts.map((l) => (
                  <span key={`pill-${l.id}`} className="arch-edge-pill" style={{ left: l.x, top: l.y }}>{view.edges[Number(l.id)].kind}{l.count > 1 ? ` ×${l.count}` : ""}</span>
                ))}
              </div>
            </div>
          )}
        </div>
        {view && layout && (
          <div className="arch-zoom" role="toolbar" title={t.arch_zoom_hint}>
            <button type="button" className="arch-btn arch-zoom-btn" aria-label={t.arch_zoom_out} title={t.arch_zoom_out} disabled={cur.scale <= MIN_ZOOM + 1e-9} onClick={zoomOut}>－</button>
            <button type="button" className="arch-zoom-pct" data-testid="arch-zoom-pct" title={t.arch_zoom_reset} onClick={zoomReset}>{Math.round(cur.scale * 100)}%</button>
            <button type="button" className="arch-btn arch-zoom-btn" aria-label={t.arch_zoom_in} title={t.arch_zoom_in} disabled={cur.scale >= MAX_ZOOM - 1e-9} onClick={zoomIn}>＋</button>
            <button type="button" className="arch-btn arch-zoom-fit" title={t.arch_zoom_hint} onClick={fitView}>{t.arch_fit}</button>
          </div>
        )}
        {tour && <TourBar tour={tour.tour} index={tour.index} playing={tour.playing} onIndex={setTourIndex} onPlaying={setTourPlaying} onClose={endTour} onOpenFile={openFile} />}
        {root && preview && preview.root === root && <FilePreview root={root} path={preview.path} line={preview.line} onClose={closePreview} />}
        </div>
        {view && (
          <SidePane open={side.open} width={side.width} onWidth={side.setWidth} onToggle={side.toggle}>
            <DetailPanel arch={view} selected={selected} onSelect={(id) => { setSelected(id); side.expand(); }} scan={scan} onExpand={enter} activePath={activePath} onPickPath={pickPath} onOpenFile={root ? openFile : undefined} />
          </SidePane>
        )}
      </div>
      {qaOpen && scan && arch && (
        <QaPanel messages={qaMessages} busy={qaBusy} stage={qaStage} elapsed={qaElapsed} onSend={ask} onCancel={cancelAsk} onPlayTour={playTour} onExport={exportQa} onClear={clearQa} onDelete={deleteQuestion} canUndo={qaUndo !== null} onUndo={undoQa} />
      )}
    </div>
  );
}
