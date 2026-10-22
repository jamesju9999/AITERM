import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { DetailPanel } from "./DetailPanel";
import { LocaleProvider } from "../../contexts/LocaleContext";
import type { Architecture } from "./model";

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
});
