import { render, screen, fireEvent, waitFor, act, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LocaleProvider } from "../../contexts/LocaleContext";

const open = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...a: unknown[]) => open(...a) }));

const archScan = vi.fn();
const archLoad = vi.fn();
const archSave = vi.fn();
const archReadFiles = vi.fn();
const archReadFile = vi.fn();
const archOpenPath = vi.fn();
vi.mock("../../ipc/architecture", () => ({
  archScan: (...a: unknown[]) => archScan(...a),
  archLoad: (...a: unknown[]) => archLoad(...a),
  archSave: (...a: unknown[]) => archSave(...a),
  archReadFiles: (...a: unknown[]) => archReadFiles(...a),
  archReadFile: (...a: unknown[]) => archReadFile(...a),
  archOpenPath: (...a: unknown[]) => archOpenPath(...a),
}));

const invokeAiComplete = vi.fn();
const abortAi = vi.fn();
vi.mock("../../ipc/ai", async (orig) => ({
  ...(await orig<typeof import("../../ipc/ai")>()),
  invokeAiComplete: (...a: unknown[]) => invokeAiComplete(...a),
  abortAi: (...a: unknown[]) => abortAi(...a),
}));

const listProviders = vi.fn();
vi.mock("../../ipc/provider", () => ({ listProviders: (...a: unknown[]) => listProviders(...a) }));

// 真正的選擇器會去查配額（Tauri invoke）；這裡只要一個能選的控制項。
vi.mock("../ModelPickerButton", () => ({
  ModelPickerButton: (p: { providers: { id: string; display_name: string }[]; selectedId: string; onChange: (id: string) => void }) => (
    <select aria-label="model" value={p.selectedId} onChange={(e) => p.onChange(e.target.value)}>
      {p.providers.map((x) => <option key={x.id} value={x.id}>{x.display_name}</option>)}
    </select>
  ),
}));

import { ArchitectureView } from "./index";
import { fitScale, layoutGraph, PAD, LANE_W, COL_GAP, HEADER_H, NODE_H_COMPACT, ROW_GAP, LANE_PAD, READABLE_SCALE, MIN_ZOOM, initialView, trueFitView } from "./layout";
import type { Architecture } from "./model";

const PROVIDERS = [
  { id: "local", display_name: "Local", is_default: true },
  { id: "cloud", display_name: "Cloud", is_default: false },
];

const SCAN = {
  root: "/p",
  files: [
    { path: "web/App.tsx", lang: "typescript", lines: 50 },
    { path: "server/routes.py", lang: "python", lines: 80 },
  ],
  edges: [],
  deps: [],
  truncated: false,
  git_head: "h1",
};

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  listProviders.mockResolvedValue(PROVIDERS);
  open.mockResolvedValue("/p");
  archLoad.mockResolvedValue(null);
  archScan.mockResolvedValue(SCAN);
  archReadFiles.mockResolvedValue([]);
  archReadFile.mockResolvedValue({ content: "const a = 1;\nexport default a;\n", truncated: false, total_bytes: 30 });
  archOpenPath.mockResolvedValue(undefined);
  abortAi.mockResolvedValue(undefined);
});

const renderView = () => render(<LocaleProvider><ArchitectureView isActive /></LocaleProvider>);

describe("ArchitectureView", () => {
  it("選資料夾後掃描並顯示模組方塊；點方塊開詳情", async () => {
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    const web = await screen.findByRole("button", { name: /web/ });
    fireEvent.click(web);
    expect(await screen.findByRole("heading", { name: "web" })).toBeInTheDocument();
  });

  it("單檔模組的「預覽檔案」開啟抽屜顯示內容；換選模組後點另一個檔案會取代；Esc 關閉", async () => {
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    fireEvent.click(await screen.findByRole("button", { name: /web/ }));
    fireEvent.click(await screen.findByRole("button", { name: /^(預覽檔案|Preview file)$/ }));
    const dlg = await screen.findByRole("dialog");
    expect(await within(dlg).findByText("export default a;")).toBeInTheDocument();
    expect(archReadFile).toHaveBeenLastCalledWith("/p", "web/App.tsx");

    archReadFile.mockResolvedValue({ content: "import flask", truncated: false, total_bytes: 12 });
    fireEvent.click(screen.getByRole("button", { name: /server/ }));
    fireEvent.click(await screen.findByRole("button", { name: /^(預覽檔案|Preview file)$/ }));
    expect(await screen.findByText("import flask")).toBeInTheDocument();
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(archReadFile).toHaveBeenLastCalledWith("/p", "server/routes.py");

    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("AI 分析成功後以分析結果取代預設分層並存檔", async () => {
    invokeAiComplete.mockResolvedValue({
      content: JSON.stringify({
        layers: [{ id: "frontend", name: "前端" }, { id: "backend", name: "後端" }],
        modules: [
          { id: "ui", layer: "frontend", name: "介面", files: ["web/"], tech: ["React"], summary: "s", dataflow: "d" },
          { id: "api", layer: "backend", name: "介面服務", files: ["server/"], tech: [], summary: "", dataflow: "" },
        ],
        edges: [],
      }),
      tool_calls: [], tool_calling_unsupported: false,
    });
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: /web/ });
    fireEvent.click(screen.getByRole("button", { name: /AI 分析|Analyze with AI/ }));
    expect(await screen.findByRole("button", { name: "介面" })).toBeInTheDocument();
    await waitFor(() => expect(archSave).toHaveBeenCalledTimes(1));
  });

  it("掃描被截斷時提示實際掃描的檔案數", async () => {
    archScan.mockResolvedValue({ ...SCAN, truncated: true });
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    expect(await screen.findByText(/只掃描了前 2 個檔案|only the first 2 files/)).toBeInTheDocument();
  });

  it("AI 未設定時保留掃描圖並提示", async () => {
    invokeAiComplete.mockRejectedValue({ kind: "not_configured" });
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: /web/ });
    fireEvent.click(screen.getByRole("button", { name: /AI 分析|Analyze with AI/ }));
    expect(await screen.findByText(/尚未設定 AI|AI is not configured/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /web/ })).toBeInTheDocument();
  });

  it("快取的 git_head 與目前不同時顯示過期提示", async () => {
    archLoad.mockResolvedValue(JSON.stringify({
      layers: [{ id: "frontend", name: "前端" }],
      modules: [{ id: "ui", layer: "frontend", name: "介面", files: ["web/App.tsx"], tech: [], summary: "", dataflow: "" }],
      edges: [], meta: { scannedAt: "", gitHead: "OLD", analyzed: true, dropped: 0 },
    }));
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    expect(await screen.findByText(/可能已過期|may be stale/)).toBeInTheDocument();
  });

  it("尚未選資料夾時顯示空狀態，且只有一個選擇資料夾按鈕", () => {
    renderView();
    expect(screen.getAllByRole("button", { name: /選擇專案資料夾|Choose project folder/ })).toHaveLength(1);
    expect(screen.queryByRole("button", { name: /AI 分析|Analyze with AI/ })).not.toBeInTheDocument();
  });

  it("選了資料夾之後，選擇按鈕也只剩工具列那一個", async () => {
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: /web/ });
    expect(screen.getAllByRole("button", { name: /選擇專案資料夾|Choose project folder/ })).toHaveLength(1);
  });

  it("AI 分析中顯示經過時間，每秒更新", async () => {
    invokeAiComplete.mockReturnValue(new Promise(() => {}));
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: /web/ });
    vi.useFakeTimers();
    try {
      fireEvent.click(screen.getByRole("button", { name: /AI 分析|Analyze with AI/ }));
      await act(async () => { await vi.advanceTimersByTimeAsync(0); });
      expect(screen.getByText(/0:00/)).toBeInTheDocument();
      await act(async () => { await vi.advanceTimersByTimeAsync(83_000); });
      expect(screen.getByText(/1:23/)).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("取消 AI 分析：呼叫 abortAi、回到原本的圖、不顯示錯誤", async () => {
    let rejectAi: (e: unknown) => void = () => {};
    invokeAiComplete.mockReturnValue(new Promise((_, rej) => { rejectAi = rej; }));
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: /web/ });
    fireEvent.click(screen.getByRole("button", { name: /AI 分析|Analyze with AI/ }));
    const cancel = await screen.findByRole("button", { name: /^(取消|Cancel)$/ });
    const streamId = invokeAiComplete.mock.calls[0][2];
    fireEvent.click(cancel);
    expect(abortAi).toHaveBeenCalledWith(streamId);
    await act(async () => { rejectAi({ kind: "model_error", message: "aborted" }); });
    expect(screen.queryByText(/分析失敗|Analysis failed/)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^(取消|Cancel)$/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /web/ })).toBeInTheDocument();
    expect(archSave).not.toHaveBeenCalled();
  });

  it("模型選擇器：預設選預設供應商，分析時把選的供應商 id 傳給 AI，並記住選擇", async () => {
    invokeAiComplete.mockResolvedValue({ content: "{}", tool_calls: [], tool_calling_unsupported: false });
    const { unmount } = renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: /web/ });
    const picker = await screen.findByRole("combobox", { name: "model" });
    expect((picker as HTMLSelectElement).value).toBe("local");
    fireEvent.change(picker, { target: { value: "cloud" } });
    fireEvent.click(screen.getByRole("button", { name: /AI 分析|Analyze with AI/ }));
    await waitFor(() => expect(invokeAiComplete).toHaveBeenCalledTimes(1));
    expect(invokeAiComplete.mock.calls[0][3]).toBe("cloud");
    unmount();

    // 重開後記得上次的選擇
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    const again = await screen.findByRole("combobox", { name: "model" });
    await waitFor(() => expect((again as HTMLSelectElement).value).toBe("cloud"));
  });

  it("記住的供應商已被刪除時退回預設", async () => {
    localStorage.setItem("aiterm_arch_provider", "gone");
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    const picker = await screen.findByRole("combobox", { name: "model" });
    await waitFor(() => expect((picker as HTMLSelectElement).value).toBe("local"));
  });
});

describe("ArchitectureView 鑽取", () => {
  const DSCAN = {
    root: "/p",
    files: [
      { path: "web/core/ui/x.ts", lang: "typescript", lines: 50 },
      { path: "web/core/ui/y.ts", lang: "typescript", lines: 50 },
      { path: "web/core/lib/z.ts", lang: "typescript", lines: 50 },
      { path: "server/routes.py", lang: "python", lines: 80 },
    ],
    edges: [{ from: "web/core/ui/x.ts", to: "web/core/lib/z.ts" }],
    deps: ["react"],
    truncated: false,
    git_head: "h1",
  };
  const AI_SUB = {
    content: JSON.stringify({
      layers: [{ id: "k", name: "核心層" }],
      modules: [{ id: "eng", layer: "k", name: "引擎", files: ["web/core/"], tech: [], summary: "", dataflow: "" }],
      edges: [],
    }),
    tool_calls: [], tool_calling_unsupported: false,
  };
  const choose = async () => {
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: "web/core" });
  };
  const nav = () => screen.queryByRole("navigation", { name: /鑽取路徑|Drill path/ });
  const drillIn = async () => {
    await choose();
    fireEvent.doubleClick(screen.getByRole("button", { name: "web/core" }));
    await screen.findByRole("button", { name: "ui" });
  };

  beforeEach(() => { archScan.mockResolvedValue(DSCAN); });

  it("雙擊模組進入內部圖並顯示麵包屑；點「全部」回到根", async () => {
    await choose();
    expect(nav()).not.toBeInTheDocument();
    fireEvent.doubleClick(screen.getByRole("button", { name: "web/core" }));
    expect(await screen.findByRole("button", { name: "ui" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "lib" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "server" })).not.toBeInTheDocument();
    expect(nav()).toHaveTextContent("web/core");
    fireEvent.click(screen.getByRole("button", { name: /^(全部|All)$/ }));
    expect(await screen.findByRole("button", { name: "server" })).toBeInTheDocument();
    expect(nav()).not.toBeInTheDocument();
  });

  it("詳情面板的「展開」鈕也能進入；單一檔案的節點沒有展開鈕且雙擊不會再鑽", async () => {
    await choose();
    fireEvent.click(screen.getByRole("button", { name: "web/core" }));
    fireEvent.click(await screen.findByRole("button", { name: /^(展開|Expand)$/ }));
    await screen.findByRole("button", { name: "ui" });
    // util 只有一個檔案
    fireEvent.click(screen.getByRole("button", { name: "lib" }));
    await screen.findByRole("heading", { name: "lib" });
    expect(screen.queryByRole("button", { name: /^(展開|Expand)$/ })).not.toBeInTheDocument();
    fireEvent.doubleClick(screen.getByRole("button", { name: "lib" }));
    expect(nav()).not.toHaveTextContent("lib");
  });

  it("切換層級時清空選取", async () => {
    await choose();
    fireEvent.click(screen.getByRole("button", { name: "web/core" }));
    await screen.findByRole("heading", { name: "web/core" });
    fireEvent.doubleClick(screen.getByRole("button", { name: "web/core" }));
    await screen.findByRole("button", { name: "ui" });
    expect(screen.queryByRole("heading", { name: "web/core" })).not.toBeInTheDocument();
  });

  it("節點超過上限時提示未顯示的數量", async () => {
    const files = Array.from({ length: 61 }, (_, i) => ({ path: `big/m/f${i}.ts`, lang: "typescript", lines: 1 }));
    archScan.mockResolvedValue({ ...DSCAN, files, edges: [] });
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    fireEvent.doubleClick(await screen.findByRole("button", { name: "big/m" }));
    expect(await screen.findByText(/另有 1 個節點未顯示|1 more nodes not shown/)).toBeInTheDocument();
    // 提示要說明可以怎麼做
    expect(screen.getByText(/AI 細分此模組.*更小的模組|refine this module with AI.*smaller module/)).toBeInTheDocument();
  });

  it("舊存檔沒有 drills 欄位也能載入並鑽取", async () => {
    archLoad.mockResolvedValue(JSON.stringify({
      layers: [{ id: "frontend", name: "前端" }],
      modules: [{ id: "web/core", layer: "frontend", name: "web/core", files: ["web/core/ui/x.ts", "web/core/ui/y.ts", "web/core/lib/z.ts"], tech: [], summary: "", dataflow: "" }],
      edges: [], meta: { scannedAt: "", gitHead: "h1", analyzed: true, dropped: 0 },
    }));
    await drillIn();
    expect(screen.getByRole("button", { name: "lib" })).toBeInTheDocument();
  });

  it("未選取模組時右側顯示技術棧總覽", async () => {
    await choose();
    expect(screen.getByText(/技術棧總覽|Tech stack overview/)).toBeInTheDocument();
    expect(screen.getByText("python")).toBeInTheDocument();
    expect(screen.getByText("react")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "web/core" }));
    await screen.findByRole("heading", { name: "web/core" });
    expect(screen.queryByText(/技術棧總覽|Tech stack overview/)).not.toBeInTheDocument();
  });

  it("AI 細分只在鑽取中出現；成功後整份存檔（含 drills），再次進入優先顯示 AI 結果", async () => {
    invokeAiComplete.mockResolvedValue(AI_SUB);
    await choose();
    expect(screen.queryByRole("button", { name: /AI 細分此模組|Refine with AI/ })).not.toBeInTheDocument();
    fireEvent.doubleClick(screen.getByRole("button", { name: "web/core" }));
    await screen.findByRole("button", { name: "ui" });
    fireEvent.click(screen.getByRole("button", { name: /AI 細分此模組|Refine with AI/ }));
    expect(await screen.findByRole("button", { name: "引擎" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "ui" })).not.toBeInTheDocument();

    // 只送該模組的檔案
    const msg = invokeAiComplete.mock.calls[0][0][0].content as string;
    expect(msg).toContain("web/core");
    expect(msg).not.toContain("server/routes.py");
    expect(archReadFiles.mock.calls[0][1].every((f: string) => f.startsWith("web/"))).toBe(true);

    await waitFor(() => expect(archSave).toHaveBeenCalledTimes(1));
    const saved = JSON.parse(archSave.mock.calls[0][1]);
    expect(Object.keys(saved.drills)).toEqual(["web/core"]);
    expect(saved.drills["web/core"].modules[0].id).toBe("eng");
    expect(saved.modules.map((m: { id: string }) => m.id).sort()).toEqual(["server", "web/core"]); // 根圖仍在

    expect(screen.getByRole("button", { name: /重新細分|Re-refine/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^(全部|All)$/ }));
    fireEvent.doubleClick(await screen.findByRole("button", { name: "web/core" }));
    expect(await screen.findByRole("button", { name: "引擎" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "ui" })).not.toBeInTheDocument();
  });

  it("AI 細分失敗時顯示錯誤、保留掃描版內部圖、不存檔", async () => {
    invokeAiComplete.mockResolvedValue({ content: "not json", tool_calls: [], tool_calling_unsupported: false });
    await drillIn();
    fireEvent.click(screen.getByRole("button", { name: /AI 細分此模組|Refine with AI/ }));
    expect(await screen.findByText(/分析失敗|Analysis failed/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "ui" })).toBeInTheDocument();
    expect(archSave).not.toHaveBeenCalled();
  });

  it("重新做根層分析時，只保留仍存在的模組的 drills", async () => {
    archLoad.mockResolvedValue(JSON.stringify({
      layers: [{ id: "frontend", name: "前端" }],
      modules: [{ id: "web/core", layer: "frontend", name: "web/core", files: ["web/core/ui/x.ts", "web/core/ui/y.ts", "web/core/lib/z.ts"], tech: [], summary: "", dataflow: "" }],
      edges: [], meta: { scannedAt: "", gitHead: "h1", analyzed: true, dropped: 0 },
      drills: { "web/core": { layers: [], modules: [], edges: [], meta: { scannedAt: "", gitHead: "h1", analyzed: true, dropped: 0 } }, gone: { layers: [], modules: [], edges: [], meta: { scannedAt: "", gitHead: "h1", analyzed: true, dropped: 0 } } },
    }));
    invokeAiComplete.mockResolvedValue({
      content: JSON.stringify({
        layers: [{ id: "f", name: "前端" }],
        modules: [
          { id: "web/core", layer: "f", name: "網頁", files: ["web/"], tech: [], summary: "", dataflow: "" },
          { id: "api", layer: "f", name: "介面服務", files: ["server/"], tech: [], summary: "", dataflow: "" },
        ],
        edges: [],
      }),
      tool_calls: [], tool_calling_unsupported: false,
    });
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: "web/core" });
    fireEvent.click(screen.getByRole("button", { name: /^(AI 分析|Analyze with AI)$/ }));
    await screen.findByRole("button", { name: "網頁" });
    await waitFor(() => expect(archSave).toHaveBeenCalledTimes(1));
    const saved = JSON.parse(archSave.mock.calls[0][1]);
    expect(Object.keys(saved.drills)).toEqual(["web/core"]);
  });
});

describe("ArchitectureView 資料流與縮放平移", () => {
  const VIEW_W = 300;
  const VIEW_H = 2000;
  const mod = (id: string, layer: string) => ({ id, layer, name: `M${id}`, files: [`${id}/x.ts`], tech: [], summary: "", dataflow: "" });
  // b 是起點；b→c 與 b→d 兩條不同的下游路徑共用起點，a→b 是上游。
  const CACHED = {
    layers: [{ id: "l1", name: "一" }, { id: "l2", name: "二" }, { id: "l3", name: "三" }],
    modules: [mod("a", "l1"), mod("b", "l2"), mod("c", "l3"), mod("d", "l3")],
    edges: [
      { from: "a", to: "b", kind: "import", source: "scan", confidence: 1 },
      { from: "b", to: "c", kind: "ipc", source: "ai", confidence: 0.9, note: "n1", evidence: { file: "b/x.ts", line: 3 } },
      { from: "b", to: "d", kind: "http", source: "ai", confidence: 0.9, note: "n2", evidence: { file: "b/x.ts", line: 9 } },
    ],
    meta: { scannedAt: "", gitHead: "h1", analyzed: true, dropped: 0 },
  };
  const LAYOUT_W = PAD * 2 + 3 * LANE_W + 2 * COL_GAP;
  const LAYOUT_H = PAD * 2 + HEADER_H + 2 * NODE_H_COMPACT + ROW_GAP + LANE_PAD;

  let origRO: typeof ResizeObserver | undefined;
  let origMM: typeof window.matchMedia | undefined;
  beforeEach(() => {
    origRO = globalThis.ResizeObserver;
    origMM = window.matchMedia;
    class RO {
      cb: (e: { contentRect: { width: number; height: number } }[]) => void;
      constructor(cb: (e: { contentRect: { width: number; height: number } }[]) => void) { this.cb = cb; }
      observe() { this.cb([{ contentRect: { width: VIEW_W, height: VIEW_H } }]); }
      disconnect() {}
      unobserve() {}
    }
    globalThis.ResizeObserver = RO as unknown as typeof ResizeObserver;
    archLoad.mockResolvedValue(JSON.stringify(CACHED));
    archScan.mockResolvedValue({ ...SCAN, files: CACHED.modules.map((m) => ({ path: `${m.id}/x.ts`, lang: "typescript", lines: 1 })) });
  });
  afterEach(() => {
    globalThis.ResizeObserver = origRO as typeof ResizeObserver;
    window.matchMedia = origMM as typeof window.matchMedia;
  });

  const setReduced = (reduced: boolean) => {
    window.matchMedia = ((q: string) => ({ matches: reduced && q.includes("reduce"), media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, onchange: null, dispatchEvent: () => false })) as unknown as typeof window.matchMedia;
  };
  const open_ = async () => {
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: "Mb" });
  };
  const nodeOf = (name: string) => document.querySelector(`.arch-node[aria-label="${name}"]`) as HTMLElement;
  const stage = () => document.querySelector(".arch-stage") as HTMLElement;
  const canvas = () => document.querySelector(".arch-canvas") as HTMLElement;
  const xf = () => {
    const m = /translate\(([-\d.e]+)px, ?([-\d.e]+)px\) scale\(([-\d.e]+)\)/.exec(stage().style.transform);
    if (!m) throw new Error(`no transform: ${stage().style.transform}`);
    return { tx: +m[1], ty: +m[2], s: +m[3] };
  };
  const onEdges = () => document.querySelectorAll('.arch-edge[data-on="true"]');
  const cards = (dir: string) => [...document.querySelectorAll(`[data-flow="${dir}"]`)] as HTMLElement[];
  // 收合的路徑卡只顯示起點與終點名稱；用終點名稱找卡。
  const card = (dir: string, end: string) => cards(dir).find((el) => el.textContent?.includes(end))!;

  it("點資料流路徑卡只高亮該路徑的連線與模組，再點一次還原", async () => {
    await open_();
    fireEvent.click(nodeOf("Mb"));
    await screen.findByRole("heading", { name: "Mb" });
    expect(onEdges()).toHaveLength(3); // 沒選路徑：與 Mb 直接相連的三條
    fireEvent.click(card("down", "Mc"));
    expect(onEdges()).toHaveLength(1);
    expect(card("down", "Mc").getAttribute("aria-pressed")).toBe("true");
    expect(nodeOf("Mc")).not.toHaveClass("arch-node-dim");
    expect(nodeOf("Md")).toHaveClass("arch-node-dim");
    expect(nodeOf("Ma")).toHaveClass("arch-node-dim");
    // 換另一條共用起點的路徑：高亮跟著換
    fireEvent.click(card("down", "Md"));
    expect(onEdges()).toHaveLength(1);
    expect(nodeOf("Md")).not.toHaveClass("arch-node-dim");
    expect(nodeOf("Mc")).toHaveClass("arch-node-dim");
    fireEvent.click(card("down", "Md"));
    expect(onEdges()).toHaveLength(3);
    expect(nodeOf("Mc")).not.toHaveClass("arch-node-dim");
  });

  it("換選取模組時清除已選路徑", async () => {
    await open_();
    fireEvent.click(nodeOf("Mb"));
    await screen.findByRole("heading", { name: "Mb" });
    fireEvent.click(card("down", "Mc"));
    expect(onEdges()).toHaveLength(1);
    fireEvent.click(nodeOf("Mb")); // 同一個模組重選也不應殘留
    fireEvent.click(nodeOf("Ma"));
    await screen.findByRole("heading", { name: "Ma" });
    expect(document.querySelector("[aria-pressed='true']")).toBeNull();
    expect(onEdges()).toHaveLength(1); // Ma 只有 a→b 一條
  });

  it("高亮連線上有光點（animateMotion）；prefers-reduced-motion 時沒有", async () => {
    setReduced(true);
    await open_();
    fireEvent.click(nodeOf("Mb"));
    await screen.findByRole("heading", { name: "Mb" });
    fireEvent.click(card("down", "Mc"));
    expect(document.querySelectorAll("animateMotion")).toHaveLength(0);
  });

  it("沒有 reduced-motion 時每條高亮連線 2 個光點，時長 1–4 秒", async () => {
    setReduced(false);
    await open_();
    fireEvent.click(nodeOf("Mb"));
    await screen.findByRole("heading", { name: "Mb" });
    fireEvent.click(card("down", "Mc"));
    const ms = [...document.querySelectorAll("animateMotion")];
    expect(ms).toHaveLength(2);
    for (const m of ms) {
      const dur = parseFloat(m.getAttribute("dur") ?? "");
      expect(dur).toBeGreaterThanOrEqual(1);
      expect(dur).toBeLessThanOrEqual(4);
    }
    expect(ms[0].getAttribute("begin")).not.toBe(ms[1].getAttribute("begin"));
  });

  it("matchMedia 不存在時（jsdom 預設）也不會壞", async () => {
    // @ts-expect-error 模擬沒有 matchMedia
    window.matchMedia = undefined;
    await open_();
    fireEvent.click(nodeOf("Mb"));
    await screen.findByRole("heading", { name: "Mb" });
    fireEvent.click(card("down", "Mc"));
    expect(onEdges()).toHaveLength(1);
  });

  it("初次載入用自動視圖（不低於易讀下限）並垂直置中；Ctrl+滾輪縮放，一般滾輪不縮放；適合視窗鈕做真適合", async () => {
    await open_();
    const iv = initialView(LAYOUT_W, LAYOUT_H, VIEW_W, VIEW_H);
    expect(fitScale(LAYOUT_W, LAYOUT_H, VIEW_W, VIEW_H)).toBeLessThan(READABLE_SCALE);
    expect(iv.scale).toBe(READABLE_SCALE);
    const v0 = xf();
    expect(v0.s).toBeCloseTo(iv.scale);
    expect(v0.tx).toBeCloseTo(iv.tx);
    expect(v0.ty).toBeCloseTo(iv.ty);

    fireEvent.wheel(canvas(), { deltaY: -100 });
    expect(xf()).toEqual(v0);

    fireEvent.wheel(canvas(), { deltaY: -100, ctrlKey: true, clientX: 100, clientY: 100 });
    const v1 = xf();
    expect(v1.s).toBeGreaterThan(v0.s);
    fireEvent.wheel(canvas(), { deltaY: 100, metaKey: true, clientX: 100, clientY: 100 });
    expect(xf().s).toBeLessThan(v1.s);

    fireEvent.wheel(canvas(), { deltaY: -100, ctrlKey: true });
    fireEvent.click(screen.getByRole("button", { name: /適合視窗|Fit to window/ }));
    const tf = trueFitView(LAYOUT_W, LAYOUT_H, VIEW_W, VIEW_H);
    expect(xf().s).toBeCloseTo(tf.scale);
    expect(xf().s).toBeLessThan(READABLE_SCALE);
    expect(xf().ty).toBeCloseTo(tf.ty);
  });

  it("背景拖曳會平移；按在卡片上不平移，且仍可點選", async () => {
    await open_();
    const v0 = xf();
    fireEvent.mouseDown(canvas(), { button: 0, clientX: 50, clientY: 50 });
    fireEvent.mouseMove(window, { clientX: 90, clientY: 80 });
    fireEvent.mouseUp(window);
    const v1 = xf();
    expect(v1.tx).toBeCloseTo(v0.tx + 40);
    expect(v1.ty).toBeCloseTo(v0.ty + 30);
    expect(v1.s).toBeCloseTo(v0.s);

    const node = nodeOf("Mb");
    fireEvent.mouseDown(node, { button: 0, clientX: 50, clientY: 50 });
    fireEvent.mouseMove(window, { clientX: 200, clientY: 200 });
    fireEvent.mouseUp(window);
    expect(xf()).toEqual(v1);
    fireEvent.click(node);
    expect(await screen.findByRole("heading", { name: "Mb" })).toBeInTheDocument();
  });

  it("名稱與層標題有完整名稱的 title", async () => {
    await open_();
    expect(nodeOf("Mb")).toHaveAttribute("title", "Mb");
    expect(document.querySelector(".arch-lane-name")).toHaveAttribute("title", "一");
  });
});

describe("ArchitectureView 稠密圖易讀性", () => {
  const VIEW_W = 700;
  const VIEW_H = 400;
  const full = (id: string, layer: string) => ({ id, layer, name: `F${id}`, files: [`${id}.ts`], tech: ["React"], summary: "有摘要", dataflow: "" });
  const file = (id: string, layer: string) => ({ id, layer, name: `F${id}`, files: [`${id}.ts`], tech: [], summary: "", dataflow: "" });
  const mods = [
    ...Array.from({ length: 30 }, (_, i) => file(`m${i}`, "l1")),
    full("hub", "l2"), ...Array.from({ length: 8 }, (_, i) => file(`n${i}`, "l2")),
    file("x1", "l3"), file("x2", "l3"), file("x3", "l3"),
  ];
  const e = (from: string, to: string) => ({ from, to, kind: "import", source: "scan", confidence: 1 });
  const edges = [
    ...Array.from({ length: 12 }, (_, i) => e(`m${i}`, "hub")), // hub 的 12 個上游
    e("hub", "x1"), e("x1", "x2"), e("x2", "x3"),               // 下游鏈：x2、x3 離 hub 2、3 步
    ...Array.from({ length: 17 }, (_, i) => e(`m${i + 13}`, `m${i + 12}`)),
    ...Array.from({ length: 8 }, (_, i) => e(`n${i}`, `m${i + 20}`)),
    ...Array.from({ length: 8 }, (_, i) => e(`n${i}`, `x${(i % 3) + 1}`)),
  ];
  const DENSE: Architecture = {
    layers: [{ id: "l1", name: "一" }, { id: "l2", name: "二" }, { id: "l3", name: "三" }],
    modules: mods, edges: edges as Architecture["edges"],
    meta: { scannedAt: "", gitHead: "h1", analyzed: false, dropped: 0 },
  };

  let origRO: typeof ResizeObserver | undefined;
  beforeEach(() => {
    origRO = globalThis.ResizeObserver;
    class RO {
      cb: (e: { contentRect: { width: number; height: number } }[]) => void;
      constructor(cb: (e: { contentRect: { width: number; height: number } }[]) => void) { this.cb = cb; }
      observe() { this.cb([{ contentRect: { width: VIEW_W, height: VIEW_H } }]); }
      disconnect() {}
      unobserve() {}
    }
    globalThis.ResizeObserver = RO as unknown as typeof ResizeObserver;
    archLoad.mockResolvedValue(JSON.stringify(DENSE));
    archScan.mockResolvedValue({ ...SCAN, files: mods.map((m) => ({ path: `${m.id}.ts`, lang: "typescript", lines: 1 })) });
  });
  afterEach(() => { globalThis.ResizeObserver = origRO as typeof ResizeObserver; });

  const open_ = async () => {
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: "Fhub" });
  };
  const nodeOf = (name: string) => document.querySelector(`.arch-node[aria-label="${name}"]`) as HTMLElement;
  const onEdges = () => document.querySelectorAll('.arch-edge[data-on="true"]');
  const pills = () => document.querySelectorAll(".arch-edge-pill");

  it("邊數 > 40 且沒選取時，連線是安靜的底圖（arch-edge-quiet）", async () => {
    expect(DENSE.edges.length).toBeGreaterThan(40);
    await open_();
    expect(document.querySelectorAll(".arch-edge-quiet")).toHaveLength(DENSE.edges.length);
    expect(onEdges()).toHaveLength(0);
    expect(pills()).toHaveLength(0);
  });

  it("選取 hub 只高亮碰到它的 13 條連線與直接鄰居，不展開整條鏈；其餘連線壓低", async () => {
    await open_();
    fireEvent.click(nodeOf("Fhub"));
    await screen.findByRole("heading", { name: "Fhub" });
    expect(onEdges()).toHaveLength(13); // 12 上游 + hub→x1；x1→x2、x2→x3 不算
    expect(document.querySelectorAll(".arch-edge-dim")).toHaveLength(DENSE.edges.length - 13);
    expect(document.querySelectorAll(".arch-edge-quiet")).toHaveLength(0);
    expect(pills()).toHaveLength(13);
    expect(nodeOf("Fm3")).not.toHaveClass("arch-node-dim");
    expect(nodeOf("Fx1")).not.toHaveClass("arch-node-dim");
    expect(nodeOf("Fx2")).toHaveClass("arch-node-dim"); // 兩步以外
    expect(nodeOf("Fx3")).toHaveClass("arch-node-dim");
    expect(nodeOf("Fm20")).toHaveClass("arch-node-dim");
  });

  it("滑過節點時一跳強調（不選取）；移開還原", async () => {
    await open_();
    fireEvent.mouseEnter(nodeOf("Fm13")); // m13→m12 與 m14→m13
    expect(onEdges()).toHaveLength(2);
    expect(nodeOf("Fm12")).not.toHaveClass("arch-node-dim");
    expect(nodeOf("Fm0")).toHaveClass("arch-node-dim");
    expect(screen.queryByRole("heading", { name: "Fm13" })).not.toBeInTheDocument();
    fireEvent.mouseLeave(nodeOf("Fm13"));
    expect(onEdges()).toHaveLength(0);
    expect(document.querySelectorAll(".arch-edge-quiet")).toHaveLength(DENSE.edges.length);
  });

  it("沒有摘要與技術標籤的檔案節點是精簡卡，完整卡維持原樣", async () => {
    await open_();
    expect(nodeOf("Fm3")).toHaveClass("arch-node-compact");
    expect(nodeOf("Fm3").style.height).toBe(`${NODE_H_COMPACT}px`);
    expect(nodeOf("Fm3").querySelector(".arch-node-sum")).toBeNull();
    expect(nodeOf("Fhub")).not.toHaveClass("arch-node-compact");
    expect(nodeOf("Fhub").querySelector(".arch-node-sum")).not.toBeNull();
  });

  it("適合視窗鈕同時考慮寬高：縮放後內容完全放進可視區；初次自動視圖不低於易讀下限", async () => {
    await open_();
    const l = layoutGraph(DENSE);
    const fit = fitScale(l.width, l.height, VIEW_W, VIEW_H);
    expect(fit).toBeLessThan(READABLE_SCALE); // fixture 要夠大，這個斷言才有鑑別力
    const parse = () => {
      const m = /translate\(([-\d.e]+)px, ?([-\d.e]+)px\) scale\(([-\d.e]+)\)/.exec((document.querySelector(".arch-stage") as HTMLElement).style.transform)!;
      return { tx: +m[1], ty: +m[2], s: +m[3] };
    };
    expect(parse().s).toBe(READABLE_SCALE);
    fireEvent.click(screen.getByRole("button", { name: /適合視窗|Fit to window/ }));
    const v = parse();
    expect(v.s).toBeCloseTo(fit);
    expect(l.height * v.s).toBeLessThanOrEqual(VIEW_H + 0.5);
    expect(l.width * v.s).toBeLessThanOrEqual(VIEW_W + 0.5);
    expect(v.tx).toBeGreaterThanOrEqual(0);
    expect(v.ty).toBeGreaterThanOrEqual(0);
  });
});

describe("ArchitectureView 很寬的圖（鑽取 11 欄）", () => {
  const VIEW_W = 1100;
  const VIEW_H = 300;
  const LAYER_IDS = Array.from({ length: 11 }, (_, i) => `d${i}`);
  // 第 2 欄 29 張（3 個子欄），其餘每欄數張：寬約 3676，真適合 ≈ 0.299，剛好低於舊的 MIN_ZOOM 0.3。
  const mods = Array.from({ length: 60 }, (_, i) => ({
    id: `n${i}`, layer: i < 29 ? "d1" : LAYER_IDS[[0, 2, 3, 4, 5, 6, 7, 8, 9, 10][i % 10]], name: `N${i}`, files: [`n${i}.ts`], tech: [], summary: "", dataflow: "",
  }));
  const WIDE: Architecture = {
    layers: LAYER_IDS.map((id, i) => ({ id, name: `第 ${i + 1} 層` })), modules: mods, edges: [],
    meta: { scannedAt: "", gitHead: "h1", analyzed: false, dropped: 0 },
  };
  let origRO: typeof ResizeObserver | undefined;
  beforeEach(() => {
    origRO = globalThis.ResizeObserver;
    class RO {
      cb: (e: { contentRect: { width: number; height: number } }[]) => void;
      constructor(cb: (e: { contentRect: { width: number; height: number } }[]) => void) { this.cb = cb; }
      observe() { this.cb([{ contentRect: { width: VIEW_W, height: VIEW_H } }]); }
      disconnect() {}
      unobserve() {}
    }
    globalThis.ResizeObserver = RO as unknown as typeof ResizeObserver;
    archLoad.mockResolvedValue(JSON.stringify(WIDE));
    archScan.mockResolvedValue({ ...SCAN, files: mods.map((m) => ({ path: `${m.id}.ts`, lang: "typescript", lines: 1 })) });
  });
  afterEach(() => { globalThis.ResizeObserver = origRO as typeof ResizeObserver; });

  const xf = () => {
    const m = /translate\(([-\d.e]+)px, ?([-\d.e]+)px\) scale\(([-\d.e]+)\)/.exec((document.querySelector(".arch-stage") as HTMLElement).style.transform)!;
    return { tx: +m[1], ty: +m[2], s: +m[3] };
  };
  const open_ = async () => {
    renderView();
    fireEvent.click(screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ }));
    await screen.findByRole("button", { name: "N0" });
  };

  it("fixture 的真適合比例確實落在舊 MIN_ZOOM 0.3 以下、新 MIN_ZOOM 以上", () => {
    const l = layoutGraph(WIDE);
    const t = Math.min(VIEW_W / l.width, VIEW_H / l.height);
    expect(t).toBeLessThan(0.3);
    expect(t).toBeGreaterThan(MIN_ZOOM);
  });

  it("適合視窗鈕：真適合，內容整個放進可視區（最後一欄不被切掉）並置中", async () => {
    await open_();
    fireEvent.click(screen.getByRole("button", { name: /適合視窗|Fit to window/ }));
    const l = layoutGraph(WIDE);
    const v = xf();
    expect(v.s).toBeCloseTo(fitScale(l.width, l.height, VIEW_W, VIEW_H));
    expect(l.width * v.s + v.tx).toBeLessThanOrEqual(VIEW_W + 0.5);
    expect(l.height * v.s + v.ty).toBeLessThanOrEqual(VIEW_H + 0.5);
  });

  it("自動初始視圖不低於 READABLE_SCALE，內容比視窗大時靠左上（不置中）", async () => {
    await open_();
    const v = xf();
    expect(v.s).toBe(READABLE_SCALE);
    expect(v.tx).toBe(0);
    expect(v.ty).toBe(0);
  });
});

describe("ArchitectureView 記住上次的專案資料夾", () => {
  const KEY = "aiterm_arch_root";
  const pickBtn = () => screen.getByRole("button", { name: /選擇專案資料夾|Choose project folder/ });

  it("掛載時若記著資料夾，自動載入（掃描該路徑），不需要再選", async () => {
    localStorage.setItem(KEY, "/saved");
    renderView();
    await waitFor(() => expect(archScan).toHaveBeenCalledWith("/saved"));
    expect(archLoad).toHaveBeenCalledWith("/saved");
    expect(await screen.findByTitle("/saved")).toBeInTheDocument();
    expect(open).not.toHaveBeenCalled();
  });

  it("沒有記著的資料夾時不掃描，顯示空狀態", () => {
    renderView();
    expect(archScan).not.toHaveBeenCalled();
    expect(pickBtn()).toBeInTheDocument();
  });

  it("選好資料夾並載入成功後存下路徑", async () => {
    open.mockResolvedValue("/chosen");
    renderView();
    fireEvent.click(pickBtn());
    await waitFor(() => expect(archScan).toHaveBeenCalledWith("/chosen"));
    await waitFor(() => expect(localStorage.getItem(KEY)).toBe("/chosen"));
  });

  it("手動選的資料夾掃描失敗時不存路徑，維持錯誤訊息", async () => {
    open.mockResolvedValue("/bad");
    archScan.mockRejectedValue("boom");
    renderView();
    fireEvent.click(pickBtn());
    await screen.findByText(/boom/);
    expect(localStorage.getItem(KEY)).toBeNull();
  });

  it("記著的資料夾已不存在（掃描失敗）：清掉記錄、顯示空狀態按鈕、沒有錯誤橫幅", async () => {
    localStorage.setItem(KEY, "/gone");
    archScan.mockRejectedValue("no such dir");
    renderView();
    await waitFor(() => expect(archScan).toHaveBeenCalledWith("/gone"));
    await waitFor(() => expect(localStorage.getItem(KEY)).toBeNull());
    expect(await screen.findByRole("button", { name: /選擇專案資料夾|Choose project folder/ })).toBeInTheDocument();
    expect(document.querySelector(".arch-status-error")).toBeNull();
    expect(screen.queryByText(/no such dir/)).not.toBeInTheDocument();
  });

  it("localStorage 丟例外也不會壞", async () => {
    const spy = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("denied"); });
    renderView();
    expect(pickBtn()).toBeInTheDocument();
    spy.mockRestore();
  });
});
