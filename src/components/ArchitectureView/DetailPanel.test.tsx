import { render, screen, fireEvent, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DetailPanel } from "./DetailPanel";
import { LocaleProvider } from "../../contexts/LocaleContext";
import type { Architecture } from "./model";
import type { ScanResult } from "../../ipc/architecture";

const arch: Architecture = {
  layers: [{ id: "a", name: "前端" }, { id: "b", name: "後端" }],
  modules: [
    { id: "web", layer: "a", name: "網頁", files: ["web/App.tsx"], tech: ["React"], summary: "畫面", dataflow: "呼叫 /api" },
    { id: "srv", layer: "b", name: "服務", files: ["srv/r.py"], tech: ["Flask"], summary: "API", dataflow: "讀 DB" },
  ],
  edges: [{ from: "web", to: "srv", kind: "http", source: "ai", confidence: 0.8, evidence: { file: "web/api.ts", line: 5 }, note: "fetch" }],
  meta: { scannedAt: "", gitHead: null, analyzed: true, dropped: 0 },
};

function setup(selected: string | null, onSelect = vi.fn()) {
  render(<LocaleProvider><DetailPanel arch={arch} selected={selected} onSelect={onSelect} /></LocaleProvider>);
  return onSelect;
}

describe("DetailPanel", () => {
  it("未選取時顯示提示", () => {
    setup(null);
    expect(screen.getByText(/點選任一模組|Click a module/)).toBeInTheDocument();
  });

  it("顯示技術、資料流、下游模組與佐證，點下游可跳轉", () => {
    const onSelect = setup("web");
    expect(screen.getByText("React")).toBeInTheDocument();
    expect(screen.getByText("呼叫 /api")).toBeInTheDocument();
    // 同一條連線也會出現在資料流路徑卡裡，這裡只看「連線」區（第一個清單）
    const edgeList = within(document.querySelector(".arch-edge-list") as HTMLElement);
    expect(edgeList.getByText(/web\/api\.ts:5/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "服務" }));
    expect(onSelect).toHaveBeenCalledWith("srv");
  });

  it("AI 連線顯示信心度百分比", () => {
    setup("web");
    expect(screen.getByText(/80%/)).toBeInTheDocument();
  });

  it("檔案清單預設只列前 8 個，可展開全部再收合", () => {
    const files = Array.from({ length: 12 }, (_, i) => `big/f${i}.ts`);
    const big: Architecture = { ...arch, modules: [{ ...arch.modules[0], files }, arch.modules[1]] };
    render(<LocaleProvider><DetailPanel arch={big} selected="web" onSelect={vi.fn()} /></LocaleProvider>);
    expect(screen.getByText("big/f7.ts")).toBeInTheDocument();
    expect(screen.queryByText("big/f8.ts")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /顯示全部 \(12\)|Show all \(12\)/ }));
    expect(screen.getByText("big/f11.ts")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /收合|Show less/ }));
    expect(screen.queryByText("big/f11.ts")).not.toBeInTheDocument();
  });

  const scan: ScanResult = {
    root: "/p",
    files: [
      { path: "web/App.tsx", lang: "typescript", lines: 10 },
      { path: "web/b.ts", lang: "typescript", lines: 10 },
      { path: "srv/r.py", lang: "python", lines: 10 },
      { path: "srv/s.py", lang: "python", lines: 10 },
    ],
    edges: [
      { from: "web/App.tsx", to: "srv/r.py" }, { from: "web/b.ts", to: "srv/r.py" },
      { from: "srv/s.py", to: "web/b.ts" }, { from: "web/App.tsx", to: "web/b.ts" },
    ],
    deps: ["react", "flask"], truncated: false, git_head: null,
  };
  const multi: Architecture = {
    ...arch,
    modules: [{ ...arch.modules[0], files: ["web/App.tsx", "web/b.ts"] }, { ...arch.modules[1], files: ["srv/r.py", "srv/s.py"] }],
  };

  it("互相引用的同種連線只顯示一筆（含 ↔），兩個方向的備註與佐證都在", () => {
    const mutual: Architecture = {
      ...multi,
      edges: [
        { from: "web", to: "srv", kind: "http", source: "ai", confidence: 0.8, evidence: { file: "web/api.ts", line: 5 }, note: "fetch" },
        { from: "srv", to: "web", kind: "http", source: "ai", confidence: 0.6, evidence: { file: "srv/cb.py", line: 9 }, note: "webhook" },
      ],
    };
    render(<LocaleProvider><DetailPanel arch={mutual} selected="web" onSelect={vi.fn()} scan={scan} /></LocaleProvider>);
    expect(screen.getAllByRole("button", { name: "服務" })).toHaveLength(1);
    expect(screen.getByText("↔")).toBeInTheDocument();
    const edgeList = within(document.querySelector(".arch-edge-list") as HTMLElement);
    expect(edgeList.getByText("fetch")).toBeInTheDocument();
    expect(edgeList.getByText("webhook")).toBeInTheDocument();
    expect(edgeList.getByText(/srv\/cb\.py:9/)).toBeInTheDocument();
  });

  it("互相引用但兩邊都沒有備註／佐證（掃描連線）：不出現孤零零的 → ←；有備註的那一邊才有方向箭頭", () => {
    const bare: Architecture = {
      ...multi,
      edges: [
        { from: "web", to: "srv", kind: "import", source: "scan", confidence: 1 },
        { from: "srv", to: "web", kind: "import", source: "scan", confidence: 1 },
      ],
    };
    const { unmount } = render(<LocaleProvider><DetailPanel arch={bare} selected="web" onSelect={vi.fn()} scan={scan} /></LocaleProvider>);
    expect(screen.getByText("↔")).toBeInTheDocument();
    expect(document.querySelectorAll(".arch-dir")).toHaveLength(0);
    expect(document.querySelectorAll(".arch-edge-line")).toHaveLength(0);
    unmount();
    const half: Architecture = { ...bare, edges: [bare.edges[0], { ...bare.edges[1], note: "回呼" }] };
    render(<LocaleProvider><DetailPanel arch={half} selected="web" onSelect={vi.fn()} scan={scan} /></LocaleProvider>);
    const dirs = [...document.querySelectorAll(".arch-dir")].map((d) => d.textContent);
    expect(dirs).toEqual(["←"]);
  });

  it("單向連線不顯示 ↔", () => {
    setup("web");
    expect(screen.queryByText("↔")).not.toBeInTheDocument();
  });

  it("顯示被外部引用／引用外部的條數", () => {
    render(<LocaleProvider><DetailPanel arch={multi} selected="web" onSelect={vi.fn()} scan={scan} /></LocaleProvider>);
    expect(screen.getByText(/被外部引用 1 條|Referenced by 1 external/)).toBeInTheDocument();
    expect(screen.getByText(/引用外部 2 條|References 2 external/)).toBeInTheDocument();
  });

  it("多檔模組有「展開」鈕，單檔模組沒有", () => {
    const onExpand = vi.fn();
    const { unmount } = render(<LocaleProvider><DetailPanel arch={multi} selected="web" onSelect={vi.fn()} scan={scan} onExpand={onExpand} /></LocaleProvider>);
    fireEvent.click(screen.getByRole("button", { name: /^(展開|Expand)$/ }));
    expect(onExpand).toHaveBeenCalledWith("web");
    unmount();
    render(<LocaleProvider><DetailPanel arch={arch} selected="web" onSelect={vi.fn()} scan={scan} onExpand={onExpand} /></LocaleProvider>);
    expect(screen.queryByRole("button", { name: /^(展開|Expand)$/ })).not.toBeInTheDocument();
  });

  it("未選取且有掃描結果時顯示概覽：語言、技術標籤、依賴、計數", () => {
    render(<LocaleProvider><DetailPanel arch={multi} selected={null} onSelect={vi.fn()} scan={scan} /></LocaleProvider>);
    expect(screen.getByText(/點選任一模組|Click a module/)).toBeInTheDocument();
    expect(screen.getByText("typescript")).toBeInTheDocument();
    expect(screen.getByText("python")).toBeInTheDocument();
    expect(screen.getByText("React")).toBeInTheDocument();
    expect(screen.getByText("flask")).toBeInTheDocument();
    expect(screen.getByText(/2 個模組・1 條連線|2 modules, 1 edges/)).toBeInTheDocument();
  });

  describe("資料流路徑", () => {
    const flowArch: Architecture = {
      layers: [{ id: "a", name: "前端" }],
      modules: ["a", "b", "c", "d"].map((id) => ({ id, layer: "a", name: `M${id}`, files: [`${id}/x.ts`], tech: [], summary: "", dataflow: "" })),
      edges: [
        { from: "a", to: "b", kind: "import", source: "scan", confidence: 1 },
        { from: "b", to: "c", kind: "ipc", source: "ai", confidence: 0.9, note: "呼叫指令", evidence: { file: "b/x.ts", line: 7 } },
        { from: "b", to: "d", kind: "http", source: "scan", confidence: 1 },
      ],
      meta: { scannedAt: "", gitHead: null, analyzed: true, dropped: 0 },
    };
    const renderFlow = (activePath: Architecture["edges"] | null, onPickPath = vi.fn()) => {
      const r = render(<LocaleProvider><DetailPanel arch={flowArch} selected="b" onSelect={vi.fn()} activePath={activePath} onPickPath={onPickPath} /></LocaleProvider>);
      return { ...r, onPickPath };
    };

    it("收合的路徑卡只有一行：起點 → … → 終點 · N 步，title 帶完整鏈", () => {
      const { container } = renderFlow(null);
      expect(container.querySelectorAll('[data-flow="up"]')).toHaveLength(1);
      const down = [...container.querySelectorAll('[data-flow="down"]')];
      expect(down).toHaveLength(2);
      const toC = down.find((el) => el.textContent?.includes("Mc"))!;
      expect(toC.textContent).toMatch(/Mb\s*→\s*Mc/);
      expect(toC.textContent).toMatch(/1 步|1 step/);
      expect(toC.getAttribute("title")).toBe("Mb → Mc");
      // 收合時不展開逐步資訊
      expect(toC.textContent).not.toContain("ipc");
      expect(toC.textContent).not.toContain("呼叫指令");
      expect(toC.textContent).not.toContain("b/x.ts:7");
      expect(toC.querySelector(".arch-step-link")).toBeNull();
      // 上游路徑：a → b
      const up = container.querySelector('[data-flow="up"]')!;
      expect(up.getAttribute("title")).toBe("Ma → Mb");
    });

    it("多步路徑收合時中間以 … 省略，步數正確", () => {
      const chain: Architecture = {
        ...flowArch,
        edges: [
          { from: "a", to: "b", kind: "import", source: "scan", confidence: 1 },
          { from: "b", to: "c", kind: "ipc", source: "ai", confidence: 0.9 },
          { from: "c", to: "d", kind: "sql", source: "scan", confidence: 1 },
        ],
      };
      const { container } = render(<LocaleProvider><DetailPanel arch={chain} selected="a" onSelect={vi.fn()} activePath={null} onPickPath={vi.fn()} /></LocaleProvider>);
      const card = container.querySelector('[data-flow="down"]')!;
      expect(card.textContent).toMatch(/Ma\s*→\s*…\s*→\s*Md/);
      expect(card.textContent).toMatch(/3 步|3 steps/);
      expect(card.getAttribute("title")).toBe("Ma → Mb → Mc → Md");
    });

    it("使用中的路徑展開成逐節點步進器：每列一個節點，列間顯示 kind、備註與 file:line", () => {
      const { container } = renderFlow(flowArch.edges.slice(1, 2)); // b→c（AI 連線）
      const active = [...container.querySelectorAll('[data-flow="down"]')].find((el) => el.getAttribute("aria-pressed") === "true")!;
      const rows = active.querySelectorAll(".arch-step-row");
      expect([...rows].map((r) => r.textContent)).toEqual(["Mb", "Mc"]);
      const links = active.querySelectorAll(".arch-step-link");
      expect(links).toHaveLength(1);
      expect(links[0].textContent).toContain("ipc");
      expect(links[0].textContent).toContain("呼叫指令");
      expect(links[0].textContent).toContain("b/x.ts:7");
      // 另一張沒被選中的卡仍然收合
      const other = [...container.querySelectorAll('[data-flow="down"]')].find((el) => el.getAttribute("aria-pressed") === "false")!;
      expect(other.querySelector(".arch-step-row")).toBeNull();
    });

    it("掃描連線的步進器列間只有 kind，沒有佐證", () => {
      const { container } = renderFlow(flowArch.edges.slice(2, 3)); // b→d（掃描）
      const link = container.querySelector('[data-flow="down"][aria-pressed="true"] .arch-step-link')!;
      expect(link.textContent).toContain("http");
      expect(link.textContent).not.toContain(":");
    });

    it("點路徑卡呼叫 onPickPath(該路徑)；activePath 對應的卡 aria-pressed", () => {
      const { container, onPickPath } = renderFlow(flowArch.edges.slice(2, 3));
      const down = [...container.querySelectorAll('[data-flow="down"]')];
      const toC = down.find((el) => el.getAttribute("title") === "Mb → Mc")!;
      const toD = down.find((el) => el.getAttribute("title") === "Mb → Md")!;
      expect(toD.getAttribute("aria-pressed")).toBe("true");
      expect(toC.getAttribute("aria-pressed")).toBe("false");
      fireEvent.click(toC);
      expect(onPickPath).toHaveBeenCalledWith([flowArch.edges[1]]);
    });

    it("沒有任何路徑時不顯示資料流路徑區", () => {
      render(<LocaleProvider><DetailPanel arch={arch} selected="srv" onSelect={vi.fn()} /></LocaleProvider>);
      expect(document.querySelector("[data-flow='down']")).toBeNull();
    });
  });
});
