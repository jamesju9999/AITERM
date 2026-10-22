import { describe, expect, it } from "vitest";
import { buildBaseGraph, guessLayer, moduleIdOf } from "./model";
import type { ScanResult } from "../../ipc/architecture";

const scan = (files: string[], edges: [string, string][] = []): ScanResult => ({
  root: "/p",
  files: files.map((path) => ({ path, lang: "typescript", lines: 10 })),
  edges: edges.map(([from, to]) => ({ from, to })),
  deps: [],
  truncated: false,
  git_head: null,
});

describe("moduleIdOf", () => {
  it("取前兩層目錄；根目錄檔案歸 (root)", () => {
    expect(moduleIdOf("src/components/A.tsx")).toBe("src/components");
    expect(moduleIdOf("src/App.tsx")).toBe("src");
    expect(moduleIdOf("main.py")).toBe("(root)");
  });
});

describe("guessLayer", () => {
  it("依路徑關鍵字猜分層", () => {
    expect(guessLayer("src/components")).toBe("frontend");
    expect(guessLayer("src-tauri/src")).toBe("backend");
    expect(guessLayer("app/models")).toBe("data");
    expect(guessLayer("scripts")).toBe("other");
  });
});

describe("buildBaseGraph", () => {
  it("把檔案 import 彙整成模組間的 scan 連線，忽略模組內部連線", () => {
    const g = buildBaseGraph(scan(
      ["src/components/A.tsx", "src/components/B.tsx", "src/ipc/x.ts"],
      [["src/components/A.tsx", "src/ipc/x.ts"], ["src/components/A.tsx", "src/components/B.tsx"]],
    ));
    expect(g.modules.map((m) => m.id).sort()).toEqual(["src/components", "src/ipc"]);
    expect(g.edges).toHaveLength(1);
    expect(g.edges[0]).toMatchObject({ from: "src/components", to: "src/ipc", kind: "import", source: "scan" });
    expect(g.modules.find((m) => m.id === "src/components")!.files).toHaveLength(2);
  });
});
