# Artifact Panel（AiPanel 落地版）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 `/ai`+`/agent` 之外的日常聊天面板（`AiPanel`/`ChatPanelShell`）能顯示 AI
生成的文件（任意 HTML，沙盒 iframe 渲染）與圖表（結構化 JSON，recharts 渲染），內容
出現時面板內部裂成「聊天欄 + Artifact 面板」兩欄。

**Architecture:** 新增一個 per-tab 的 `ArtifactPanelContext`；擴充 `src/lib/markdown.tsx`
既有的 mermaid fenced-code-block 機制，辨識 ` ```artifact-html `/` ```artifact-chart `
並登記進 context；`ChatPanelShell` 訂閱 context，有內容時仿照 `DesignView.tsx` 的手刻
拖拉分割版型裂成兩欄。HTML 用 `<iframe sandbox="allow-scripts">`（不給
`allow-same-origin`）隔離；圖表用 `recharts` + 專案 `dataviz` skill 的驗證過色票。

**Tech Stack:** React 19 + TypeScript（`src/`），Vitest + React Testing Library，新增
依賴 `recharts`（`nanoid` 已是既有依賴，圖示重用 `src/components/Icons.tsx` 既有的
`FileTextIcon`/`ChartIcon`）。

---

## 這份計畫涵蓋的 spec 章節

對應 `docs/superpowers/specs/2026-09-01-artifact-panel-design.md`：

- 「1. 整體架構」→ Task 2（context）+ Task 9（`ChatPanelShell` 訂閱與分割）
- 「2. AI 怎麼標記」→ Task 8（`markdown.tsx` 擴充）+ Task 7（`ArtifactBlockCard`）
- 「3. 兩種內容類型的渲染與安全性」→ Task 3（`chartPalette`）、Task 4
  （`ArtifactHtmlFrame` 沙盒）、Task 5（`ArtifactChart`）、Task 6（`ArtifactPanel` 分派）
- 「4. 面板 UI / 佈局」→ Task 9
- 「5. 串流行為」→ 不需要額外程式碼，是 Task 8 對 remark 既有解析行為的自然結果，
  Task 8 的測試會驗證這一點
- 「明確排除」章節 → 本計畫刻意不涵蓋 `DesignView`/`DatabaseAiChat` 整合、即時串流
  預覽、CSP 網路層限制，符合 spec 排除範圍

---

### Task 1: 新增 `recharts` 依賴

**Files:**
- Modify: `package.json`
- Modify: `package-lock.json`（`npm install` 自動更新，不用手動編輯）

- [ ] **Step 1: 安裝 `recharts`**

Run: `npm install recharts`

Expected: `package.json` 的 `dependencies` 區塊新增一行（依專案既有的 `^` 版本區間
慣例），插在 `"react-router-dom"` 之後、`"rehype-katex"` 之前（維持字母順序）：

```json
    "react-router-dom": "^7.14.0",
    "recharts": "^3.x.x",
    "rehype-katex": "^7.0.1",
```

（實際安裝到的確切版本號以 `npm install` 實際解析結果為準，不要手動改成別的版本。）

- [ ] **Step 2: 確認型別檢查與既有測試沒有被裝新套件影響**

Run: `npx tsc -b`
Expected: 無錯誤（新依賴目前還沒被任何程式碼引用，不應該產生新的型別錯誤）。

Run: `npm run test -- --run`
Expected: 全部既有測試維持通過（這一步只是裝依賴，還沒改任何程式碼）。

- [ ] **Step 3: Commit**

```bash
git add package.json package-lock.json
git commit -m "chore: add recharts for the Artifact Panel chart renderer"
```

---

### Task 2: `ArtifactPanelContext`

**Files:**
- Create: `src/contexts/ArtifactPanelContext.tsx`
- Test: `src/contexts/ArtifactPanelContext.test.tsx`

- [ ] **Step 1: 寫測試（會因為檔案還不存在而編譯失敗）**

建立 `src/contexts/ArtifactPanelContext.test.tsx`：

```tsx
import { describe, expect, it } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ArtifactPanelProvider, useArtifactPanel } from "./ArtifactPanelContext";

function Probe() {
  const { activeArtifact, showArtifact, clearArtifact } = useArtifactPanel();
  return (
    <div>
      <div data-testid="active">
        {activeArtifact ? `${activeArtifact.kind}:${activeArtifact.title}` : "none"}
      </div>
      <button onClick={() => showArtifact({ id: "1", kind: "html", title: "Brief", content: "<p>hi</p>" })}>
        show-a
      </button>
      <button onClick={() => showArtifact({ id: "2", kind: "chart", title: "Sales", content: "{}" })}>
        show-b
      </button>
      <button onClick={clearArtifact}>clear</button>
    </div>
  );
}

describe("ArtifactPanelContext", () => {
  it("starts with no active artifact", () => {
    render(<ArtifactPanelProvider><Probe /></ArtifactPanelProvider>);
    expect(screen.getByTestId("active")).toHaveTextContent("none");
  });

  it("showArtifact sets the active artifact", () => {
    render(<ArtifactPanelProvider><Probe /></ArtifactPanelProvider>);
    fireEvent.click(screen.getByText("show-a"));
    expect(screen.getByTestId("active")).toHaveTextContent("html:Brief");
  });

  it("a second showArtifact call replaces the first, not stacks", () => {
    render(<ArtifactPanelProvider><Probe /></ArtifactPanelProvider>);
    fireEvent.click(screen.getByText("show-a"));
    fireEvent.click(screen.getByText("show-b"));
    expect(screen.getByTestId("active")).toHaveTextContent("chart:Sales");
  });

  it("clearArtifact resets to none", () => {
    render(<ArtifactPanelProvider><Probe /></ArtifactPanelProvider>);
    fireEvent.click(screen.getByText("show-a"));
    fireEvent.click(screen.getByText("clear"));
    expect(screen.getByTestId("active")).toHaveTextContent("none");
  });

  it("useArtifactPanel throws when used outside a provider", () => {
    function Bare() {
      useArtifactPanel();
      return null;
    }
    expect(() => render(<Bare />)).toThrow(
      "useArtifactPanel must be used within an ArtifactPanelProvider",
    );
  });
});
```

- [ ] **Step 2: 執行測試，確認因為模組不存在而失敗**

Run: `npx vitest run src/contexts/ArtifactPanelContext.test.tsx`
Expected: 失敗，錯誤訊息包含 `Failed to resolve import "./ArtifactPanelContext"`。

- [ ] **Step 3: 實作**

建立 `src/contexts/ArtifactPanelContext.tsx`：

```tsx
import { createContext, useCallback, useContext, useState, type ReactNode } from "react";

export type ArtifactKind = "html" | "chart";

export interface Artifact {
  id: string;
  kind: ArtifactKind;
  title: string;
  /** kind="html" 時是完整 HTML 字串；kind="chart" 時是未解析的 JSON 字串
   *（由渲染面板自己 parse，見 ArtifactPanel.tsx）。 */
  content: string;
}

interface ArtifactPanelState {
  activeArtifact: Artifact | null;
  showArtifact: (artifact: Artifact) => void;
  clearArtifact: () => void;
}

const ArtifactPanelContext = createContext<ArtifactPanelState | null>(null);

export function ArtifactPanelProvider({ children }: { children: ReactNode }) {
  const [activeArtifact, setActiveArtifact] = useState<Artifact | null>(null);

  const showArtifact = useCallback((artifact: Artifact) => {
    setActiveArtifact(artifact);
  }, []);

  const clearArtifact = useCallback(() => {
    setActiveArtifact(null);
  }, []);

  return (
    <ArtifactPanelContext.Provider value={{ activeArtifact, showArtifact, clearArtifact }}>
      {children}
    </ArtifactPanelContext.Provider>
  );
}

export function useArtifactPanel(): ArtifactPanelState {
  const ctx = useContext(ArtifactPanelContext);
  if (!ctx) {
    throw new Error("useArtifactPanel must be used within an ArtifactPanelProvider");
  }
  return ctx;
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npx vitest run src/contexts/ArtifactPanelContext.test.tsx`
Expected: 5 個測試全數通過。

- [ ] **Step 5: Commit**

```bash
git add src/contexts/ArtifactPanelContext.tsx src/contexts/ArtifactPanelContext.test.tsx
git commit -m "feat(artifact-panel): add per-tab ArtifactPanelContext"
```

---

### Task 3: `chartPalette.ts`

色票數值逐字抄自 `dataviz` skill 的 `references/palette.md`（已驗證過的預設色票，
分類色 8 色固定順序、循序色單一藍色相 7 階、狀態色淺深不分色、chrome/ink 淺深各自
一組）——**不要自己調整任何 hex 值**，需要換色系時要照那份 skill 的流程重新驗證。

**Files:**
- Create: `src/lib/chartPalette.ts`
- Test: `src/lib/chartPalette.test.ts`

- [ ] **Step 1: 寫測試**

建立 `src/lib/chartPalette.test.ts`：

```ts
import { describe, expect, it } from "vitest";
import { CHART_PALETTE_LIGHT, CHART_PALETTE_DARK } from "./chartPalette";

const HEX = /^#[0-9a-f]{6}$/i;

describe("chartPalette", () => {
  it("light and dark categorical palettes each have exactly 8 distinct hex colors", () => {
    for (const palette of [CHART_PALETTE_LIGHT, CHART_PALETTE_DARK]) {
      expect(palette.categorical).toHaveLength(8);
      for (const hex of palette.categorical) expect(hex).toMatch(HEX);
      expect(new Set(palette.categorical).size).toBe(8);
    }
  });

  it("sequential ramp is 7 distinct hex steps, same across light and dark", () => {
    expect(CHART_PALETTE_LIGHT.sequential).toHaveLength(7);
    expect(CHART_PALETTE_LIGHT.sequential).toEqual(CHART_PALETTE_DARK.sequential);
    expect(new Set(CHART_PALETTE_LIGHT.sequential).size).toBe(7);
  });

  it("status colors are identical in light and dark (fixed, never themed)", () => {
    expect(CHART_PALETTE_LIGHT.status).toEqual(CHART_PALETTE_DARK.status);
  });

  it("light and dark use different surface/text tokens", () => {
    expect(CHART_PALETTE_LIGHT.surface).not.toBe(CHART_PALETTE_DARK.surface);
    expect(CHART_PALETTE_LIGHT.textPrimary).not.toBe(CHART_PALETTE_DARK.textPrimary);
  });
});
```

- [ ] **Step 2: 執行測試，確認因為模組不存在而失敗**

Run: `npx vitest run src/lib/chartPalette.test.ts`
Expected: 失敗，`Failed to resolve import "./chartPalette"`。

- [ ] **Step 3: 實作**

建立 `src/lib/chartPalette.ts`：

```ts
/**
 * 色票數值逐字抄自 `dataviz` skill 的 references/palette.md（已驗證過的預設色票）。
 * 不要手動調整這裡的任何 hex 值——換色系要照那份 skill 的流程重新驗證後才能改。
 */

export interface ThemeColors {
  categorical: string[];
  sequential: string[];
  status: { good: string; warning: string; serious: string; critical: string };
  surface: string;
  textPrimary: string;
  textSecondary: string;
  muted: string;
  gridline: string;
  baseline: string;
}

export const CHART_PALETTE_LIGHT: ThemeColors = {
  categorical: [
    "#2a78d6", // 1 blue
    "#eb6834", // 2 orange
    "#1baf7a", // 3 aqua
    "#eda100", // 4 yellow
    "#e87ba4", // 5 magenta
    "#008300", // 6 green
    "#4a3aa7", // 7 violet
    "#e34948", // 8 red
  ],
  sequential: ["#cde2fb", "#9ec5f4", "#6da7ec", "#3987e5", "#256abf", "#184f95", "#0d366b"],
  status: { good: "#0ca30c", warning: "#fab219", serious: "#ec835a", critical: "#d03b3b" },
  surface: "#fcfcfb",
  textPrimary: "#0b0b0b",
  textSecondary: "#52514e",
  muted: "#898781",
  gridline: "#e1e0d9",
  baseline: "#c3c2b7",
};

export const CHART_PALETTE_DARK: ThemeColors = {
  categorical: [
    "#3987e5", // 1 blue
    "#d95926", // 2 orange
    "#199e70", // 3 aqua
    "#c98500", // 4 yellow
    "#d55181", // 5 magenta
    "#008300", // 6 green
    "#9085e9", // 7 violet
    "#e66767", // 8 red
  ],
  sequential: ["#cde2fb", "#9ec5f4", "#6da7ec", "#3987e5", "#256abf", "#184f95", "#0d366b"],
  status: { good: "#0ca30c", warning: "#fab219", serious: "#ec835a", critical: "#d03b3b" },
  surface: "#1a1a19",
  textPrimary: "#ffffff",
  textSecondary: "#c3c2b7",
  muted: "#898781",
  gridline: "#2c2c2a",
  baseline: "#383835",
};
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npx vitest run src/lib/chartPalette.test.ts`
Expected: 4 個測試全數通過。

- [ ] **Step 5: Commit**

```bash
git add src/lib/chartPalette.ts src/lib/chartPalette.test.ts
git commit -m "feat(artifact-panel): add validated light/dark chart palette"
```

---

### Task 4: `ArtifactHtmlFrame`（沙盒 iframe）

**Files:**
- Create: `src/components/ArtifactPanel/ArtifactHtmlFrame.tsx`
- Test: `src/components/ArtifactPanel/ArtifactHtmlFrame.test.tsx`

- [ ] **Step 1: 寫測試**

建立 `src/components/ArtifactPanel/ArtifactHtmlFrame.test.tsx`：

```tsx
import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { ArtifactHtmlFrame } from "./ArtifactHtmlFrame";

describe("ArtifactHtmlFrame", () => {
  it("renders an iframe with the given HTML as srcDoc and the given title", () => {
    const { container } = render(<ArtifactHtmlFrame html="<p>hello</p>" title="Brief" />);
    const frame = container.querySelector("iframe");
    expect(frame).not.toBeNull();
    expect(frame).toHaveAttribute("srcdoc", "<p>hello</p>");
    expect(frame).toHaveAttribute("title", "Brief");
  });

  // 這是整份設計最不能退讓的安全底線：allow-scripts 沒有搭配
  // allow-same-origin，沙盒才會把 iframe 隔成不透明來源，內容碰不到主視窗
  // 的 DOM/localStorage/Tauri IPC。獨立寫一個測試，防止未來被「順手」加回
  // allow-same-origin。
  it("sandbox allows scripts but never allow-same-origin", () => {
    const { container } = render(<ArtifactHtmlFrame html="<script>1</script>" title="x" />);
    const frame = container.querySelector("iframe")!;
    const tokens = (frame.getAttribute("sandbox") ?? "").split(/\s+/).filter(Boolean);
    expect(tokens).toContain("allow-scripts");
    expect(tokens).not.toContain("allow-same-origin");
  });
});
```

- [ ] **Step 2: 執行測試，確認因為模組不存在而失敗**

Run: `npx vitest run src/components/ArtifactPanel/ArtifactHtmlFrame.test.tsx`
Expected: 失敗，`Failed to resolve import "./ArtifactHtmlFrame"`。

- [ ] **Step 3: 實作**

建立 `src/components/ArtifactPanel/ArtifactHtmlFrame.tsx`：

```tsx
interface ArtifactHtmlFrameProps {
  html: string;
  title: string;
}

/**
 * 用 sandbox iframe 隔離渲染 AI 生成的任意 HTML。允許跑 JS（allow-scripts），
 * 但絕對不給 allow-same-origin——這樣瀏覽器會把這個 iframe 當成獨立的不透明
 * 來源，裡面的 JS 完全碰不到主視窗的 DOM/localStorage，更不可能碰到 Tauri
 * 的 IPC bridge。這個組合是刻意的，不是遺漏，見
 * docs/superpowers/specs/2026-09-01-artifact-panel-design.md 的「安全性」一節。
 */
export function ArtifactHtmlFrame({ html, title }: ArtifactHtmlFrameProps) {
  return (
    <iframe
      className="aiterm-artifact-html-frame"
      title={title}
      srcDoc={html}
      sandbox="allow-scripts"
    />
  );
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npx vitest run src/components/ArtifactPanel/ArtifactHtmlFrame.test.tsx`
Expected: 2 個測試全數通過。

- [ ] **Step 5: Commit**

```bash
git add src/components/ArtifactPanel/ArtifactHtmlFrame.tsx src/components/ArtifactPanel/ArtifactHtmlFrame.test.tsx
git commit -m "feat(artifact-panel): render AI-generated HTML in a sandboxed iframe"
```

---

### Task 5: `ArtifactChart`（recharts 渲染）

**Files:**
- Create: `src/components/ArtifactPanel/ArtifactChart.tsx`
- Test: `src/components/ArtifactPanel/ArtifactChart.test.tsx`

- [ ] **Step 1: 寫測試**

建立 `src/components/ArtifactPanel/ArtifactChart.test.tsx`：

```tsx
import { describe, expect, it, beforeAll } from "vitest";
import { render } from "@testing-library/react";
import { ArtifactChart, type ChartSpec } from "./ArtifactChart";

// recharts 的 ResponsiveContainer 需要 ResizeObserver、且容器要有非零尺寸才會
// 畫出 SVG 子元素——jsdom 兩者都沒有內建，這個 repo 也沒有全域 polyfill（見
// src/test-setup.ts），所以每個會渲染 recharts 的測試檔都要自己補，做法比照
// src/components/TerminalView.closeGuard.test.tsx 既有的區域性 polyfill 慣例。
beforeAll(() => {
  class FakeResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;
  Element.prototype.getBoundingClientRect = () =>
    ({ width: 600, height: 320, top: 0, left: 0, bottom: 320, right: 600, x: 0, y: 0, toJSON() {} }) as DOMRect;
});

const barSpec: ChartSpec = {
  type: "bar",
  data: [
    { month: "Jan", sales: 120 },
    { month: "Feb", sales: 150 },
  ],
  xKey: "month",
  series: [{ key: "sales", label: "Sales" }],
};

describe("ArtifactChart", () => {
  it("renders a bar chart without throwing", () => {
    const { container } = render(<ArtifactChart spec={barSpec} />);
    expect(container.querySelector("svg")).not.toBeNull();
  });

  it("renders a line chart without throwing", () => {
    const lineSpec: ChartSpec = { ...barSpec, type: "line" };
    const { container } = render(<ArtifactChart spec={lineSpec} />);
    expect(container.querySelector("svg")).not.toBeNull();
  });

  it("renders a pie chart without throwing", () => {
    const pieSpec: ChartSpec = {
      type: "pie",
      data: [
        { label: "A", value: 10 },
        { label: "B", value: 20 },
      ],
      xKey: "label",
      series: [{ key: "value", label: "Value" }],
    };
    const { container } = render(<ArtifactChart spec={pieSpec} />);
    expect(container.querySelector("svg")).not.toBeNull();
  });

  it("does not render a legend for a single series", () => {
    const { container } = render(<ArtifactChart spec={barSpec} />);
    expect(container.querySelector(".recharts-legend-wrapper")).toBeNull();
  });

  it("renders a legend when there are two or more series", () => {
    const twoSeriesSpec: ChartSpec = {
      ...barSpec,
      series: [
        { key: "sales", label: "Sales" },
        { key: "returns", label: "Returns" },
      ],
      data: [{ month: "Jan", sales: 120, returns: 5 }],
    };
    const { container } = render(<ArtifactChart spec={twoSeriesSpec} />);
    expect(container.querySelector(".recharts-legend-wrapper")).not.toBeNull();
  });
});
```

- [ ] **Step 2: 執行測試，確認因為模組不存在而失敗**

Run: `npx vitest run src/components/ArtifactPanel/ArtifactChart.test.tsx`
Expected: 失敗，`Failed to resolve import "./ArtifactChart"`。

- [ ] **Step 3: 實作**

建立 `src/components/ArtifactPanel/ArtifactChart.tsx`：

```tsx
import { useMemo } from "react";
import {
  ResponsiveContainer,
  BarChart,
  Bar,
  LineChart,
  Line,
  PieChart,
  Pie,
  Cell,
  CartesianGrid,
  XAxis,
  YAxis,
  Tooltip,
  Legend,
} from "recharts";
import { CHART_PALETTE_LIGHT, CHART_PALETTE_DARK } from "../../lib/chartPalette";

export interface ChartSeriesSpec {
  key: string;
  label: string;
}

export interface ChartSpec {
  type: "bar" | "line" | "pie";
  title?: string;
  data: Record<string, unknown>[];
  xKey: string;
  series: ChartSeriesSpec[];
}

interface ArtifactChartProps {
  spec: ChartSpec;
}

/** 沿用 AgentStatusBar.css 既有的多主題分桶慣例：只有明確的 "light" 主題算
 *  淺色，其他（dark/nord/dracula/未設定）一律當深色處理。 */
function isDarkSurface(): boolean {
  return document.documentElement.getAttribute("data-theme") !== "light";
}

export function ArtifactChart({ spec }: ArtifactChartProps) {
  const palette = useMemo(() => (isDarkSurface() ? CHART_PALETTE_DARK : CHART_PALETTE_LIGHT), []);

  if (spec.type === "pie") {
    const seriesKey = spec.series[0]?.key ?? "value";
    return (
      <ResponsiveContainer width="100%" height={320}>
        <PieChart>
          <Pie data={spec.data} dataKey={seriesKey} nameKey={spec.xKey} label>
            {spec.data.map((_, i) => (
              <Cell key={i} fill={palette.categorical[i % palette.categorical.length]} />
            ))}
          </Pie>
          <Tooltip />
          {spec.series.length >= 2 && <Legend />}
        </PieChart>
      </ResponsiveContainer>
    );
  }

  const ChartComponent = spec.type === "line" ? LineChart : BarChart;
  const SeriesComponent = spec.type === "line" ? Line : Bar;

  return (
    <ResponsiveContainer width="100%" height={320}>
      <ChartComponent data={spec.data}>
        <CartesianGrid stroke={palette.gridline} strokeDasharray="3 3" />
        <XAxis dataKey={spec.xKey} stroke={palette.muted} />
        <YAxis stroke={palette.muted} />
        <Tooltip
          contentStyle={{
            background: palette.surface,
            border: `1px solid ${palette.baseline}`,
            color: palette.textPrimary,
          }}
        />
        {spec.series.length >= 2 && <Legend />}
        {spec.series.map((s, i) => (
          <SeriesComponent
            key={s.key}
            dataKey={s.key}
            name={s.label}
            fill={palette.categorical[i % palette.categorical.length]}
            stroke={palette.categorical[i % palette.categorical.length]}
          />
        ))}
      </ChartComponent>
    </ResponsiveContainer>
  );
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npx vitest run src/components/ArtifactPanel/ArtifactChart.test.tsx`
Expected: 5 個測試全數通過。

- [ ] **Step 5: Commit**

```bash
git add src/components/ArtifactPanel/ArtifactChart.tsx src/components/ArtifactPanel/ArtifactChart.test.tsx
git commit -m "feat(artifact-panel): add recharts-based chart renderer"
```

---

### Task 6: `ArtifactPanel`（依 kind 分派 + header）

**Files:**
- Create: `src/components/ArtifactPanel/ArtifactPanel.tsx`
- Create: `src/components/ArtifactPanel/ArtifactPanel.css`
- Test: `src/components/ArtifactPanel/ArtifactPanel.test.tsx`

- [ ] **Step 1: 寫測試**

建立 `src/components/ArtifactPanel/ArtifactPanel.test.tsx`：

```tsx
import { useEffect } from "react";
import { describe, expect, it } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import {
  ArtifactPanelProvider,
  useArtifactPanel,
  type Artifact,
} from "../../contexts/ArtifactPanelContext";
import { ArtifactPanel } from "./ArtifactPanel";

function ShowOnMount({ artifact }: { artifact: Artifact }) {
  const { showArtifact } = useArtifactPanel();
  useEffect(() => {
    showArtifact(artifact);
  }, [artifact, showArtifact]);
  return null;
}

describe("ArtifactPanel", () => {
  it("renders nothing when there is no active artifact", () => {
    const { container } = render(
      <ArtifactPanelProvider>
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    expect(container.querySelector(".aiterm-artifact-panel")).toBeNull();
  });

  it("renders an html artifact inside an iframe with the artifact's title", () => {
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "1", kind: "html", title: "Brief", content: "<p>hi</p>" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByText("Brief")).toBeInTheDocument();
    expect(document.querySelector("iframe")).not.toBeNull();
  });

  it("shows an error message when chart content is not valid JSON", () => {
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "2", kind: "chart", title: "Bad", content: "not json" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByText("圖表資料格式錯誤，無法解析。")).toBeInTheDocument();
  });

  it("clicking close clears the active artifact", () => {
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "1", kind: "html", title: "Brief", content: "<p>hi</p>" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    fireEvent.click(screen.getByTitle("關閉文件面板"));
    expect(screen.queryByText("Brief")).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 執行測試，確認因為模組不存在而失敗**

Run: `npx vitest run src/components/ArtifactPanel/ArtifactPanel.test.tsx`
Expected: 失敗，`Failed to resolve import "./ArtifactPanel"`。

- [ ] **Step 3: 實作**

建立 `src/components/ArtifactPanel/ArtifactPanel.css`：

```css
.aiterm-artifact-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  background: var(--bg-glass-solid, #0f111c);
  border-left: 1px solid var(--border-color, #2a2a2a);
}

.aiterm-artifact-panel__header {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-color, #2a2a2a);
  flex-shrink: 0;
}

.aiterm-artifact-panel__title {
  flex: 1;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary, #c3c2b7);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.aiterm-artifact-panel__close {
  background: transparent;
  border: none;
  color: var(--text-secondary, #c3c2b7);
  cursor: pointer;
  font-size: 13px;
  padding: 2px 6px;
}

.aiterm-artifact-panel__body {
  flex: 1;
  overflow: auto;
  min-height: 0;
}

.aiterm-artifact-panel__error {
  padding: 16px;
  color: var(--text-secondary, #c3c2b7);
  font-size: 13px;
}

.aiterm-artifact-html-frame {
  width: 100%;
  height: 100%;
  border: none;
  background: #fff;
}
```

建立 `src/components/ArtifactPanel/ArtifactPanel.tsx`：

```tsx
import type { ReactNode } from "react";
import { FileTextIcon, ChartIcon } from "../Icons";
import { useArtifactPanel } from "../../contexts/ArtifactPanelContext";
import { ArtifactHtmlFrame } from "./ArtifactHtmlFrame";
import { ArtifactChart, type ChartSpec } from "./ArtifactChart";
import "./ArtifactPanel.css";

export function ArtifactPanel() {
  const { activeArtifact, clearArtifact } = useArtifactPanel();
  if (!activeArtifact) return null;

  let body: ReactNode;
  if (activeArtifact.kind === "html") {
    body = <ArtifactHtmlFrame html={activeArtifact.content} title={activeArtifact.title} />;
  } else {
    let spec: ChartSpec | null = null;
    try {
      spec = JSON.parse(activeArtifact.content) as ChartSpec;
    } catch {
      spec = null;
    }
    body = spec ? (
      <ArtifactChart spec={spec} />
    ) : (
      <div className="aiterm-artifact-panel__error">圖表資料格式錯誤，無法解析。</div>
    );
  }

  return (
    <div className="aiterm-artifact-panel">
      <div className="aiterm-artifact-panel__header">
        {activeArtifact.kind === "html" ? <FileTextIcon size={15} /> : <ChartIcon size={15} />}
        <span className="aiterm-artifact-panel__title">{activeArtifact.title}</span>
        <button
          type="button"
          className="aiterm-artifact-panel__close"
          onClick={clearArtifact}
          title="關閉文件面板"
        >
          ✕
        </button>
      </div>
      <div className="aiterm-artifact-panel__body">{body}</div>
    </div>
  );
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npx vitest run src/components/ArtifactPanel/ArtifactPanel.test.tsx`
Expected: 4 個測試全數通過。

- [ ] **Step 5: Commit**

```bash
git add src/components/ArtifactPanel/ArtifactPanel.tsx src/components/ArtifactPanel/ArtifactPanel.css src/components/ArtifactPanel/ArtifactPanel.test.tsx
git commit -m "feat(artifact-panel): add ArtifactPanel dispatcher + header"
```

---

### Task 7: `ArtifactBlockCard`（聊天泡泡裡的精簡卡片）

**Files:**
- Create: `src/components/ArtifactPanel/ArtifactBlockCard.tsx`
- Create: `src/components/ArtifactPanel/ArtifactBlockCard.css`
- Test: `src/components/ArtifactPanel/ArtifactBlockCard.test.tsx`

- [ ] **Step 1: 寫測試**

建立 `src/components/ArtifactPanel/ArtifactBlockCard.test.tsx`：

```tsx
import { describe, expect, it } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ArtifactPanelProvider, useArtifactPanel } from "../../contexts/ArtifactPanelContext";
import { ArtifactBlockCard } from "./ArtifactBlockCard";

function ActiveTitle() {
  const { activeArtifact } = useArtifactPanel();
  return <div data-testid="active-title">{activeArtifact?.title ?? "none"}</div>;
}

describe("ArtifactBlockCard", () => {
  it("registers itself into the artifact panel on mount", () => {
    render(
      <ArtifactPanelProvider>
        <ArtifactBlockCard kind="html" content="<title>Brief</title><p>hi</p>" />
        <ActiveTitle />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByTestId("active-title")).toHaveTextContent("Brief");
  });

  it("falls back to a generic title when html has no <title>", () => {
    render(
      <ArtifactPanelProvider>
        <ArtifactBlockCard kind="html" content="<p>hi</p>" />
        <ActiveTitle />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByTestId("active-title")).toHaveTextContent("文件");
  });

  it("reads the title from a chart spec's JSON", () => {
    render(
      <ArtifactPanelProvider>
        <ArtifactBlockCard kind="chart" content='{"title":"Sales by Month"}' />
        <ActiveTitle />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByTestId("active-title")).toHaveTextContent("Sales by Month");
  });

  it("falls back to a generic title when chart content is not valid JSON", () => {
    render(
      <ArtifactPanelProvider>
        <ArtifactBlockCard kind="chart" content="not json" />
        <ActiveTitle />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByTestId("active-title")).toHaveTextContent("圖表");
  });

  it("clicking the card re-shows the artifact", () => {
    render(
      <ArtifactPanelProvider>
        <ArtifactBlockCard kind="html" content="<title>Brief</title>" />
        <ActiveTitle />
      </ArtifactPanelProvider>,
    );
    fireEvent.click(screen.getByRole("button"));
    expect(screen.getByTestId("active-title")).toHaveTextContent("Brief");
  });
});
```

- [ ] **Step 2: 執行測試，確認因為模組不存在而失敗**

Run: `npx vitest run src/components/ArtifactPanel/ArtifactBlockCard.test.tsx`
Expected: 失敗，`Failed to resolve import "./ArtifactBlockCard"`。

- [ ] **Step 3: 實作**

建立 `src/components/ArtifactPanel/ArtifactBlockCard.css`：

```css
.aiterm-artifact-card {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  margin-top: 4px;
  background: var(--bg-glass-solid, #0f111c);
  border: 1px solid var(--border-color, #2a2a2a);
  border-radius: 8px;
  color: var(--text-primary, #e6e6e6);
  font-size: 12px;
  cursor: pointer;
}

.aiterm-artifact-card:hover {
  border-color: var(--accent, #a855f7);
}

.aiterm-artifact-card__title {
  font-weight: 600;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.aiterm-artifact-card__kind {
  color: var(--text-secondary, #c3c2b7);
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}
```

建立 `src/components/ArtifactPanel/ArtifactBlockCard.tsx`：

```tsx
import { useEffect, useRef } from "react";
import { nanoid } from "nanoid";
import { FileTextIcon, ChartIcon } from "../Icons";
import { useArtifactPanel, type ArtifactKind } from "../../contexts/ArtifactPanelContext";
import "./ArtifactBlockCard.css";

interface ArtifactBlockCardProps {
  kind: ArtifactKind;
  content: string;
}

function guessTitle(kind: ArtifactKind, content: string): string {
  if (kind === "chart") {
    try {
      const parsed = JSON.parse(content) as { title?: string };
      if (typeof parsed.title === "string" && parsed.title.trim()) return parsed.title.trim();
    } catch {
      // 不是合法 JSON，落到下面的預設標題。
    }
    return "圖表";
  }
  const match = content.match(/<title>([^<]*)<\/title>/i);
  return match?.[1]?.trim() || "文件";
}

/**
 * 聊天泡泡裡代表一個 artifact 區塊的精簡卡片。掛載時透過 useEffect（不能在
 * 渲染期間呼叫，React 不允許在渲染一個元件時觸發另一個元件的狀態更新）把
 * 內容登記進 ArtifactPanelContext，讓側邊的 ArtifactPanel 顯示出來；點卡片
 * 可以重新叫出來（例如使用者切走去看別的 artifact 之後想切回來）。
 */
export function ArtifactBlockCard({ kind, content }: ArtifactBlockCardProps) {
  const { showArtifact } = useArtifactPanel();
  const idRef = useRef(nanoid());
  const title = guessTitle(kind, content);

  useEffect(() => {
    showArtifact({ id: idRef.current, kind, title, content });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kind, content, title]);

  return (
    <button
      type="button"
      className="aiterm-artifact-card"
      onClick={() => showArtifact({ id: idRef.current, kind, title, content })}
    >
      {kind === "html" ? <FileTextIcon size={14} /> : <ChartIcon size={14} />}
      <span className="aiterm-artifact-card__title">{title}</span>
      <span className="aiterm-artifact-card__kind">{kind === "html" ? "HTML" : "Chart"}</span>
    </button>
  );
}
```

- [ ] **Step 4: 執行測試，確認通過**

Run: `npx vitest run src/components/ArtifactPanel/ArtifactBlockCard.test.tsx`
Expected: 5 個測試全數通過。

- [ ] **Step 5: Commit**

```bash
git add src/components/ArtifactPanel/ArtifactBlockCard.tsx src/components/ArtifactPanel/ArtifactBlockCard.css src/components/ArtifactPanel/ArtifactBlockCard.test.tsx
git commit -m "feat(artifact-panel): add ArtifactBlockCard chat-bubble placeholder"
```

---

### Task 8: 擴充 `markdown.tsx` 的 fenced-code-block 協定

**Files:**
- Modify: `src/lib/markdown.tsx`
- Create: `src/lib/markdown.artifact.test.tsx`

（用新檔名 `markdown.artifact.test.tsx` 而不是找既有的 markdown 測試檔案去加，
因為這批測試需要 `ArtifactPanelProvider` 包裹、跟既有 markdown 測試的 render 方式
不同，分開一個檔案職責更清楚。）

- [ ] **Step 1: 寫測試**

建立 `src/lib/markdown.artifact.test.tsx`：

```tsx
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ArtifactPanelProvider } from "../contexts/ArtifactPanelContext";
import { MarkdownText } from "./markdown";

function renderWithProvider(text: string) {
  return render(
    <ArtifactPanelProvider>
      <MarkdownText text={text} />
    </ArtifactPanelProvider>,
  );
}

describe("MarkdownText artifact fenced blocks", () => {
  it("renders a complete artifact-html fenced block as a card, not raw code", () => {
    const text = "before\n\n```artifact-html\n<title>Brief</title><p>hi</p>\n```\n\nafter";
    renderWithProvider(text);
    expect(screen.getByText("Brief")).toBeInTheDocument();
    expect(screen.queryByText(/<title>/)).not.toBeInTheDocument();
  });

  it("renders a complete artifact-chart fenced block as a card", () => {
    const text = '```artifact-chart\n{"title":"Sales"}\n```';
    renderWithProvider(text);
    expect(screen.getByText("Sales")).toBeInTheDocument();
  });

  it("an unclosed artifact-html fence does not render a card (still mid-stream)", () => {
    const text = "```artifact-html\n<title>Brief</title>";
    renderWithProvider(text);
    expect(screen.queryByText("Brief")).not.toBeInTheDocument();
  });

  it("a plain mermaid block still renders inline as before (regression check)", () => {
    const text = '```mermaid\npie title x\n"a" : 1\n```';
    const { container } = renderWithProvider(text);
    expect(container.querySelector(".aiterm-artifact-card")).toBeNull();
  });
});
```

- [ ] **Step 2: 執行測試，確認因為協定還沒實作而失敗**

Run: `npx vitest run src/lib/markdown.artifact.test.tsx`
Expected: 前兩個測試失敗（`artifact-html`/`artifact-chart` 目前會被當成一般
` ```code ``` ` 區塊印出原始文字，找不到 `Brief`/`Sales` 這兩個字，只有含
`<title>` 字面文字的原始碼），後兩個測試通過（本來就沒有卡片、mermaid 行為不變）。

- [ ] **Step 3: 實作**

修改 `src/lib/markdown.tsx`：

1. 在既有的 `import { MermaidBlock } from "../components/MermaidBlock";`
   （第 97 行）之後新增一行：

```tsx
import { ArtifactBlockCard } from "../components/ArtifactPanel/ArtifactBlockCard";
```

2. 把 `code` renderer（第 108-113 行）：

```tsx
          code({ node, className, children, ...props }) {
            const match = /language-(\w+)/.exec(className || "");
            
            if (match && match[1].toLowerCase() === "mermaid") {
              return <MermaidBlock chart={String(children)} />;
            }
```

改成：

```tsx
          code({ node, className, children, ...props }) {
            const match = /language-(\w+)/.exec(className || "");
            
            if (match && match[1].toLowerCase() === "mermaid") {
              return <MermaidBlock chart={String(children)} />;
            }
            if (match && match[1].toLowerCase() === "artifact-html") {
              return <ArtifactBlockCard kind="html" content={String(children)} />;
            }
            if (match && match[1].toLowerCase() === "artifact-chart") {
              return <ArtifactBlockCard kind="chart" content={String(children)} />;
            }
```

（`code` renderer 其餘部分——`isInline` 判斷、`<pre>`/`<code>` fallback——完全
不變。）

- [ ] **Step 4: 執行測試，確認全部通過**

Run: `npx vitest run src/lib/markdown.artifact.test.tsx`
Expected: 4 個測試全數通過。

- [ ] **Step 5: 執行 markdown.tsx 既有的其他測試，確認沒有回歸**

Run: `npx vitest run src/lib/markdown.test.tsx`（若這個檔案不存在，改成
`npx vitest run src/lib` 執行整個 `src/lib` 目錄下的測試，確認沒有測試因為
這次改動而變紅）
Expected: 全部通過。

- [ ] **Step 6: Commit**

```bash
git add src/lib/markdown.tsx src/lib/markdown.artifact.test.tsx
git commit -m "feat(artifact-panel): recognize artifact-html/artifact-chart fenced blocks"
```

---

### Task 9: `ChatPanelShell` 訂閱 context 並裂成兩欄

**Files:**
- Modify: `src/components/ChatPanel/ChatPanelShell.tsx`（整個檔案的變動幅度大，
  下面直接給出完整新檔案內容，取代整個檔案）
- Modify: `src/components/ChatPanel/styles.css`（新增規則，不改動既有規則）
- Modify: `src/components/ChatPanel/ChatPanelShell.test.tsx`（在既有檔案尾端新增測試）

- [ ] **Step 1: 在既有測試檔尾端新增測試（會失敗，因為分割行為還沒實作）**

在 `src/components/ChatPanel/ChatPanelShell.test.tsx` 檔案最後一個 `it(...)` 區塊
（`allowEmptySubmit lets Enter submit with empty text...`）之後、`describe` 收尾的
`});` 之前，插入：

```tsx
  it("renders a two-column split with the artifact panel when a message contains an artifact-html block", () => {
    const messages = [
      { role: "assistant" as const, content: "```artifact-html\n<title>Brief</title><p>hi</p>\n```" },
    ];
    render(<ChatPanelShell {...base({ messages })} />);
    expect(screen.getByText("Brief")).toBeInTheDocument();
    expect(document.querySelector(".aiterm-ai-panel--split")).not.toBeNull();
    expect(document.querySelector("iframe")).not.toBeNull();
  });

  it("does not render the split layout when no message has an artifact block", () => {
    render(<ChatPanelShell {...base()} />);
    expect(document.querySelector(".aiterm-ai-panel--split")).toBeNull();
    expect(document.querySelector(".aiterm-artifact-panel")).toBeNull();
  });

  it("closing the artifact panel collapses back to a single column", () => {
    const messages = [
      { role: "assistant" as const, content: "```artifact-html\n<title>Brief</title>\n```" },
    ];
    render(<ChatPanelShell {...base({ messages })} />);
    fireEvent.click(screen.getByTitle("關閉文件面板"));
    expect(document.querySelector(".aiterm-ai-panel--split")).toBeNull();
  });
```

- [ ] **Step 2: 執行測試，確認新增的 3 個測試失敗、既有測試維持通過**

Run: `npx vitest run src/components/ChatPanel/ChatPanelShell.test.tsx`
Expected: 新增的 3 個測試 FAIL（`.aiterm-ai-panel--split` 這個 class 目前不存在、
`artifact-html` 目前還是印成原始文字，找不到 "Brief"），這個檔案裡原本就有的其他
測試維持 PASS。

- [ ] **Step 3: 用下面完整內容取代整個 `src/components/ChatPanel/ChatPanelShell.tsx`**

```tsx
import {
  useEffect, useRef, useState, useCallback,
  type KeyboardEvent, type ReactNode, type PointerEvent, type MouseEvent,
} from "react";
import type { AiError, ToolFallbackReason } from "../../ipc/ai";
import type { SubmitShortcut } from "../../ipc/config";
import type { McpChatMessage, McpChatSession } from "../../types/chat";
import { useLocale } from "../../contexts/LocaleContext";
import { MessageList } from "../AiPanel/MessageList";
import { ModeHint, type PanelMode } from "../AiPanel/ModeHint";
import { MaximizeIcon, MinimizeIcon, ZapIcon } from "../Icons";
import { ArtifactPanelProvider, useArtifactPanel } from "../../contexts/ArtifactPanelContext";
import { ArtifactPanel } from "../ArtifactPanel/ArtifactPanel";
import "./styles.css";

const MIN_WIDTH = 280;
const MAX_WIDTH_RATIO = 0.75;
const STORAGE_WIDTH_KEY = "aiterm-panel-width";
const MIN_CHAT_COLUMN_WIDTH = 220;
const MIN_ARTIFACT_COLUMN_WIDTH = 260;

function loadSavedWidth(): number {
  try {
    const v = localStorage.getItem(STORAGE_WIDTH_KEY);
    if (v) return Math.max(MIN_WIDTH, parseInt(v, 10));
  } catch { /* ignore */ }
  return 420;
}

export interface ChatPanelShellProps {
  isOpen: boolean;
  onClose: () => void;

  messages: McpChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  thinkingLabel: string | null;
  error: AiError | string | null;
  onRetry: () => void;
  onExecuteCommand: (cmd: string) => void;

  agentMode: boolean;
  onToggleAgentMode: () => void;
  onSend: (text: string) => void;
  onSubmitAgent: (text: string) => void;
  mode: PanelMode;
  maxAgentSteps: number;
  mcpToolCount?: number;

  agentRunning: boolean;
  agentPhase: "thinking" | "running";
  agentStep: number;
  onAbortAgent: () => void;

  providerName: string;
  onOpenProviderPalette: () => void;
  /** provider 徽章旁的插槽——呼叫端應該塞 <QuotaBadge> 進來（AiPanel 就是這樣接的）。 */
  headerBadge?: ReactNode;

  sessions: McpChatSession[];
  onLoadSession: (s: McpChatSession) => void;
  onNewChat: () => void;
  onDeleteSession: (id: string) => void;

  toolFallbackReason?: ToolFallbackReason | null;

  /** 送出鍵的觸發鍵——設定裡的 submit_shortcut，預設 Enter 送出。 */
  submitShortcut?: SubmitShortcut;
  /** 文字是空的也允許送出（例如本機面板：只有附件、沒打字也能送）。 */
  allowEmptySubmit?: boolean;
  /** 貼上事件轉發給呼叫端（例如本機面板用它接住剪貼簿裡的檔案）。 */
  onPaste?: (e: React.ClipboardEvent<HTMLTextAreaElement>) => void;
  /** 輸入框前面的額外控制項（例如本機面板的附加檔案迴紋針按鈕）。 */
  inputPrefixControls?: ReactNode;

  /** 貼在 agent-mode 開關旁那一排（輸入框上方）——AiPanel 把 MCP 開關放這裡。
   *  跟 `inputPrefixControls`（貼在輸入框「裡面」，textarea 左側）是不同位置，別搞混。 */
  extraInputControls?: ReactNode;
  /** 插在整個輸入區塊「之上」（ModeHint／agent 狀態列之後、輸入框之前）——
   *  AiPanel 放附件 pills、隱藏的檔案 input、卡住提示。 */
  extraAboveInput?: ReactNode;
  /** Windows 上背景毛玻璃看不清楚，要改成不透明樣式（見 styles.css）；預設 false。 */
  isWindows?: boolean;
  /** 外部強制禁用輸入區（例如唯讀連線）；跟既有的 isStreaming/agentRunning
   *  禁用邏輯是 OR 關係——三者任一為真就禁用，預設 false 不影響原本行為。 */
  inputDisabled?: boolean;

  onDragOver?: (e: React.DragEvent<HTMLDivElement>) => void;
  onDrop?: (e: React.DragEvent<HTMLDivElement>) => void;
}

/** 對外仍然只有一個 ChatPanelShell——ArtifactPanelProvider 包在這一層，讓每個
 *  分頁各自的 ChatPanelShell 實例都有自己獨立的 artifact 狀態（見
 *  docs/superpowers/specs/2026-09-01-artifact-panel-design.md「per-tab」段落）。
 *  Provider 一定要包在「消費它的元件」外面，所以實際內容拆到 Inner 元件。 */
export function ChatPanelShell(props: ChatPanelShellProps) {
  return (
    <ArtifactPanelProvider>
      <ChatPanelShellInner {...props} />
    </ArtifactPanelProvider>
  );
}

function ChatPanelShellInner({
  isOpen,
  onClose,
  messages,
  streamBuf,
  isStreaming,
  thinkingLabel,
  error,
  onRetry,
  onExecuteCommand,
  agentMode,
  onToggleAgentMode,
  onSend,
  onSubmitAgent,
  mode,
  maxAgentSteps,
  mcpToolCount = 0,
  agentRunning,
  agentPhase,
  agentStep,
  onAbortAgent,
  providerName,
  onOpenProviderPalette,
  headerBadge,
  sessions,
  onLoadSession,
  onNewChat,
  onDeleteSession,
  toolFallbackReason,
  submitShortcut = "enter",
  allowEmptySubmit = false,
  onPaste,
  inputPrefixControls,
  extraInputControls,
  extraAboveInput,
  isWindows = false,
  inputDisabled = false,
  onDragOver,
  onDrop,
}: ChatPanelShellProps) {
  const { t } = useLocale();
  const { activeArtifact } = useArtifactPanel();
  const [input, setInput] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);

  // 有 artifact 顯示時，視覺上一律當成「已展開」——不是把 expanded 這個
  // state 本身改掉（使用者手動按過的展開偏好要保留），只是讓寬度/樣式判斷
  // 多一個 OR 條件。artifact 收掉後會自動回到使用者原本手動設定的展開狀態。
  const effectiveExpanded = expanded || !!activeArtifact;

  // ── Resize（面板整體寬度，收合時） ─────────────────────────────────────────
  const [panelWidth, setPanelWidth] = useState(loadSavedWidth);
  const isDraggingRef = useRef(false);
  const dragStartXRef = useRef(0);
  const dragStartWidthRef = useRef(0);

  const onResizePointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    isDraggingRef.current = true;
    dragStartXRef.current = e.clientX;
    dragStartWidthRef.current = panelWidth;
    (e.currentTarget as HTMLDivElement).setPointerCapture(e.pointerId);
  };

  const onResizePointerMove = (e: PointerEvent<HTMLDivElement>) => {
    if (!isDraggingRef.current) return;
    const delta = dragStartXRef.current - e.clientX; // drag left → wider
    const maxWidth = Math.floor(window.innerWidth * MAX_WIDTH_RATIO);
    const next = Math.max(MIN_WIDTH, Math.min(maxWidth, dragStartWidthRef.current + delta));
    setPanelWidth(next);
  };

  const onResizePointerUp = () => {
    if (!isDraggingRef.current) return;
    isDraggingRef.current = false;
    setPanelWidth((w) => {
      try { localStorage.setItem(STORAGE_WIDTH_KEY, String(w)); } catch { /* ignore */ }
      return w;
    });
  };

  // ── Resize（聊天欄 vs Artifact 面板的內部分割，有 artifact 時） ───────────────
  // 做法比照 src/components/DesignView/DesignView.tsx 既有的手刻拖拉分割：
  // 用容器的 getBoundingClientRect() 算出滑鼠絕對位置對應的左欄寬度，而不是
  // 累加 delta——這樣邏輯簡單、也是這個 repo 既有分割版型的一致寫法。
  const [chatColumnWidth, setChatColumnWidth] = useState(320);
  const [isArtifactResizing, setIsArtifactResizing] = useState(false);
  const splitContainerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!isArtifactResizing) {
      document.body.style.userSelect = "";
      return;
    }
    document.body.style.userSelect = "none";
    const onMouseMove = (e: globalThis.MouseEvent) => {
      if (!splitContainerRef.current) return;
      const rect = splitContainerRef.current.getBoundingClientRect();
      const newWidth = e.clientX - rect.left;
      const constrained = Math.max(
        MIN_CHAT_COLUMN_WIDTH,
        Math.min(newWidth, rect.width - MIN_ARTIFACT_COLUMN_WIDTH),
      );
      setChatColumnWidth(constrained);
    };
    const onMouseUp = () => setIsArtifactResizing(false);
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      document.body.style.userSelect = "";
    };
  }, [isArtifactResizing]);

  useEffect(() => {
    if (isOpen) textareaRef.current?.focus();
  }, [isOpen]);

  useEffect(() => {
    const onPrefill = (e: Event) => {
      const detail = (e as CustomEvent).detail as { text: string };
      if (detail?.text) setInput(detail.text);
    };
    window.addEventListener("aiterm:prefill-chat", onPrefill);
    return () => window.removeEventListener("aiterm:prefill-chat", onPrefill);
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") { e.preventDefault(); onClose(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isOpen, onClose]);

  const isDisabled = isStreaming || agentRunning || inputDisabled;

  const submit = useCallback(() => {
    const text = input.trim();
    if ((!text && !allowEmptySubmit) || isDisabled) return;
    setInput("");
    if (agentMode) {
      onSubmitAgent(text);
    } else {
      onSend(text);
    }
  }, [input, allowEmptySubmit, isDisabled, agentMode, onSubmitAgent, onSend]);

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter") {
      const shouldSubmit =
        (submitShortcut === "enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) ||
        (submitShortcut === "shift-enter" && e.shiftKey && !e.ctrlKey) ||
        (submitShortcut === "ctrl-enter" && (e.ctrlKey || e.metaKey) && !e.shiftKey);
      if (shouldSubmit) { e.preventDefault(); submit(); }
    }
  };

  const panelClass = [
    "aiterm-ai-panel",
    isOpen ? "" : "aiterm-ai-panel-hidden",
    // Windows can't blur the terminal behind the glass panel — see styles.css.
    isWindows ? "aiterm-ai-panel--solid" : "",
    effectiveExpanded ? "aiterm-ai-panel--expanded" : "",
    activeArtifact ? "aiterm-ai-panel--split" : "",
  ].filter(Boolean).join(" ");

  const onArtifactResizeMouseDown = (e: MouseEvent<HTMLDivElement>) => {
    e.preventDefault();
    setIsArtifactResizing(true);
  };

  return (
    <div
      className={panelClass}
      aria-hidden={!isOpen}
      style={{ width: effectiveExpanded ? "100%" : `${panelWidth}px` }}
      onDragOver={onDragOver}
      onDrop={onDrop}
      ref={splitContainerRef}
    >
      {/* Resize handle on the left edge — 滿版時左邊沒有終端機可以讓，收起來。 */}
      {!effectiveExpanded && (
        <div
          className="aiterm-panel-resize-handle"
          onPointerDown={onResizePointerDown}
          onPointerMove={onResizePointerMove}
          onPointerUp={onResizePointerUp}
          title="拖曳調整寬度"
        />
      )}

      <div
        className="aiterm-ai-panel-chat-column"
        style={activeArtifact ? { width: `${chatColumnWidth}px`, flexShrink: 0, flexGrow: 0 } : { flex: 1 }}
      >
        <div className="aiterm-ai-panel-header">
          <span className="aiterm-ai-panel-title" style={{ background: 'var(--accent-gradient)', WebkitBackgroundClip: 'text', WebkitTextFillColor: 'transparent', display: 'flex', alignItems: 'center', gap: '6px', fontWeight: 700, fontSize: '14px' }}>
            ✨ AITerm AI Studio
          </span>
          <button
            type="button"
            className="aiterm-ai-panel-provider-badge"
            onClick={onOpenProviderPalette}
            title="切換 Provider"
          >
            {providerName || "(no provider)"}
            {headerBadge}
          </button>
          <div style={{ display: "flex", gap: "8px" }}>
            <button
              type="button"
              className={`aiterm-ai-panel-clear-btn aiterm-ai-panel-icon-btn${effectiveExpanded ? " aiterm-ai-panel-clear-btn--active" : ""}`}
              onClick={() => setExpanded((e) => !e)}
              title={effectiveExpanded ? "縮小面板" : "放大面板"}
            >
              {effectiveExpanded ? <MinimizeIcon size={15} /> : <MaximizeIcon size={15} />}
            </button>
            <button
              type="button"
              className={`aiterm-ai-panel-clear-btn${historyOpen ? " aiterm-ai-panel-clear-btn--active" : ""}`}
              onClick={() => setHistoryOpen((o) => !o)}
              title="對話歷史"
            >
              📋
            </button>
            <button
              type="button"
              className="aiterm-ai-panel-clear-btn"
              onClick={() => { onNewChat(); setHistoryOpen(false); }}
              disabled={isDisabled}
              title="清空當前對話"
            >
              🗑 New Chat
            </button>
            <button
              type="button"
              className="aiterm-ai-panel-clear-btn"
              onClick={onClose}
              title="關閉面板 (Esc)"
            >
              ✕
            </button>
          </div>
        </div>

        {/* History side panel */}
        {historyOpen && (
          <div className="aiterm-history-panel">
            <div className="aiterm-history-panel__header">
              <span className="aiterm-history-panel__title">對話歷史</span>
            </div>
            <div className="aiterm-history-panel__list">
              {sessions.length === 0 && (
                <div className="aiterm-history-panel__empty">尚無歷史記錄</div>
              )}
              {[...sessions].reverse().map((s) => (
                <div
                  key={s.id}
                  className="aiterm-history-panel__item"
                  onClick={() => { onLoadSession(s); setHistoryOpen(false); }}
                >
                  <div className="aiterm-history-panel__item-content">
                    <div className="aiterm-history-panel__item-title">{s.title}</div>
                    <div className="aiterm-history-panel__item-date">
                      {new Date(s.savedAt).toLocaleDateString("zh-TW", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" })}
                    </div>
                  </div>
                  <button
                    type="button"
                    className="aiterm-history-panel__item-del"
                    title="刪除此對話"
                    onClick={(e) => { e.stopPropagation(); onDeleteSession(s.id); }}
                  >
                    ×
                  </button>
                </div>
              ))}
            </div>
          </div>
        )}

        <MessageList
          messages={messages}
          streamBuf={streamBuf}
          isStreaming={isStreaming}
          thinkingLabel={thinkingLabel}
          error={error}
          onExecuteCommand={onExecuteCommand}
          onRetry={onRetry}
        />

        {/* 這個憑證無法使用原生工具呼叫時，後端會自動改用「工具描述注入系統提示」
            的文字協定。工具照樣能跑，但切換方案不該靜默發生。 */}
        {toolFallbackReason && (
          <div className="aiterm-mode-hint aiterm-mode-hint--degraded">
            <span aria-hidden="true">⚠</span>
            <span>
              {toolFallbackReason === "subscription_billing"
                ? t.ai_tool_fallback_billing
                : t.ai_tool_fallback_unsupported}
            </span>
          </div>
        )}

        {/* Agent 跑起來之後由狀態列接手（它有步驟數與中止鈕），兩條堆在一起是噪音。 */}
        {!agentRunning && (
          <ModeHint mode={mode} maxAgentSteps={maxAgentSteps} mcpToolCount={mcpToolCount} />
        )}

        {agentRunning && (
          <div className="aiterm-agent-status">
            <span
              className={`aiterm-agent-status__spinner aiterm-agent-status__spinner--${agentPhase}`}
              aria-hidden="true"
            >
              {agentPhase === "thinking" ? "⟳" : "▶"}
            </span>
            <span>
              {agentPhase === "thinking" ? t.ai_agent_thinking : t.ai_agent_executing}
              {" "}步驟 {agentStep}/{maxAgentSteps >= 9999 ? "∞" : maxAgentSteps}
            </span>
            <button
              type="button"
              className="aiterm-agent-status__stop"
              onClick={onAbortAgent}
              title="停止"
            >
              ■
            </button>
          </div>
        )}

        {extraAboveInput}

        <div className="aiterm-ai-panel-input-area">
          <div style={{ display: "flex", gap: "6px", alignItems: "center" }}>
            <button
              type="button"
              className={`aiterm-agent-toggle${agentMode ? " aiterm-agent-toggle--on" : ""}`}
              onClick={onToggleAgentMode}
              title={agentMode ? "停用 Agent 模式" : "啟用 Agent 模式（AI 自動執行指令迭代）"}
              disabled={isDisabled}
            >
              <ZapIcon size={14} isFilled={agentMode} />
            </button>
            {extraInputControls}
          </div>
          <div className="aiterm-input-pill-container">
            {inputPrefixControls}
            <textarea
              ref={textareaRef}
              className="aiterm-ai-panel-input"
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              onPaste={onPaste}
              placeholder={
                agentRunning ? "Agent 執行中…" :
                agentMode ? "目標... (Enter)" :
                isStreaming ? "等待 AI 回覆..." : "Ask AI anything..."
              }
              rows={1}
              disabled={isDisabled}
            />
            <button
              type="button"
              className="aiterm-ai-panel-send-btn aiterm-btn aiterm-btn--primary aiterm-btn--icon"
              onClick={submit}
              disabled={isDisabled || input.trim() === ""}
              title="送出"
            >
              ▲
            </button>
          </div>
        </div>
      </div>

      {activeArtifact && (
        <>
          <div
            className="aiterm-artifact-resizer"
            onMouseDown={onArtifactResizeMouseDown}
            title="拖曳調整寬度"
          />
          <ArtifactPanel />
        </>
      )}
    </div>
  );
}
```

- [ ] **Step 4: 在 `src/components/ChatPanel/styles.css` 尾端新增以下規則**（不改動
  既有規則，直接附加在檔案最後）：

```css
/* ── Artifact Panel split（有 AI 生成的文件/圖表時，面板內部裂成兩欄） ────── */
.aiterm-ai-panel--split {
  flex-direction: row;
}

.aiterm-ai-panel-chat-column {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.aiterm-artifact-resizer {
  width: 6px;
  cursor: col-resize;
  background-color: transparent;
  transition: background-color 0.2s;
  flex-shrink: 0;
}

.aiterm-artifact-resizer:hover {
  background-color: var(--accent, #a855f7);
}
```

- [ ] **Step 5: 執行測試，確認全部通過**

Run: `npx vitest run src/components/ChatPanel/ChatPanelShell.test.tsx`
Expected: 全部測試（既有的 + 這次新增的 3 個）通過。

- [ ] **Step 6: 型別檢查整個專案**

Run: `npx tsc -b`
Expected: 無錯誤。

- [ ] **Step 7: 執行完整前端測試套件，確認沒有引入任何回歸**

Run: `npm run test -- --run`
Expected: 全部通過。

- [ ] **Step 8: Commit**

```bash
git add src/components/ChatPanel/ChatPanelShell.tsx src/components/ChatPanel/styles.css src/components/ChatPanel/ChatPanelShell.test.tsx
git commit -m "feat(artifact-panel): split ChatPanelShell into chat + artifact columns"
```

---

## 完成後的驗證（無法自動化，需要真機手動確認）

九個 Task 都完成、`cargo` 不涉及（這次是純前端功能）、`npx tsc -b` 與
`npm run test -- --run` 都綠燈之後，程式碼層面已符合 spec。以下是 spec 裡明確
標註「需要真機驗證」的部分：

1. 手動在聊天輸入框貼一段包含 ` ```artifact-html ` 或 ` ```artifact-chart `
   fenced block 的訊息（因為這個里程碑還沒教模型主動輸出這個協定，見 spec
   「對既有程式碼的影響」最後一點），確認面板真的裂成兩欄、拖拉分割手感正常、
   不同視窗寬度下終端機仍留有合理空間。
2. 確認 iframe 沙盒真的擋下對主視窗的存取：餵一段會嘗試存取
   `window.parent`/`window.top`/`window.__TAURI__` 的測試 HTML 進
   `artifact-html`，人工確認這些存取全部失敗/回傳 `undefined`（跨來源不透明
   iframe 底下，`window.parent`/`window.top` 存取本身不會拋錯，但讀不到真正
   內容；`window.__TAURI__` 應該完全不存在）。
3. 在淺色/深色（及 nord/dracula）主題下切換，確認 `ArtifactChart` 的
   `isDarkSurface()` 判斷與色票挑選符合預期（只有明確切到 "light" 主題時走
   淺色色票）。
4. 真的請 AI 生成一段圖表 artifact，人工核對畫出來的圖跟 `dataviz` skill 的
   規範（分類色順序、圖例、hover tooltip）視覺上是否正確——單元測試只驗證
   「有沒有畫出 SVG」和「色票有沒有用對」，沒有驗證實際視覺呈現。
