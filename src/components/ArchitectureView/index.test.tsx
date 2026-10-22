import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { LocaleProvider } from "../../contexts/LocaleContext";

const open = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: (...a: unknown[]) => open(...a) }));

const archScan = vi.fn();
const archLoad = vi.fn();
const archSave = vi.fn();
const archReadFiles = vi.fn();
vi.mock("../../ipc/architecture", () => ({
  archScan: (...a: unknown[]) => archScan(...a),
  archLoad: (...a: unknown[]) => archLoad(...a),
  archSave: (...a: unknown[]) => archSave(...a),
  archReadFiles: (...a: unknown[]) => archReadFiles(...a),
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
