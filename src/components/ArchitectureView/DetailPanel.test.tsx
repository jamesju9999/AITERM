import { render, screen, fireEvent } from "@testing-library/react";
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
    expect(screen.getByText(/web\/api\.ts:5/)).toBeInTheDocument();
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
    expect(screen.getByText("fetch")).toBeInTheDocument();
    expect(screen.getByText("webhook")).toBeInTheDocument();
    expect(screen.getByText(/srv\/cb\.py:9/)).toBeInTheDocument();
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
});
