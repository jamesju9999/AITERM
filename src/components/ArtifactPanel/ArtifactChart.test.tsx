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

  // 規範允許折線圖下方鋪 ~10% 透明度的區域填色（"a wash, never a saturated
  // block"）。這是唯一不扭曲數值的加層次方式——立體陰影會讓讀者不確定該讀
  // 長條的前緣還是後緣。
  it("washes the area under a line at low opacity", () => {
    const { container } = render(<ArtifactChart spec={{ ...barSpec, type: "line" }} />);
    const area = container.querySelector(".recharts-area-area");
    expect(area).not.toBeNull();
    expect(Number(area!.getAttribute("fill-opacity"))).toBeLessThanOrEqual(0.15);
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

  // 圓餅圖的「扇形」本身就是類別維度——一個序列就有多個身分要區分。沒有圖例
  // 又沒有名稱標籤的話，顏色等於沒有意義。這跟長條/折線「單一序列不需要圖例」
  // 的規則不同，因為那裡的身分是序列、標題已經說完了。
  it("always shows a legend for a pie, even with one series", () => {
    const pieSpec: ChartSpec = {
      type: "pie",
      data: [{ k: "A", v: 4 }, { k: "B", v: 1 }, { k: "C", v: 2 }],
      xKey: "k",
      series: [{ key: "v", label: "數量" }],
    };
    const { container } = render(<ArtifactChart spec={pieSpec} />);
    expect(container.querySelector(".recharts-legend-wrapper")).not.toBeNull();
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

  // 上面那些「有沒有畫出 svg」的斷言擋不住一整類靜默失效：recharts 是靠掃描
  // children 找出格線/座標軸/圖例的，實作把這些共用元件放進一個 <>...</>
  // fragment 裡重用，如果 recharts 不會攤平 fragment，格線與座標軸會安靜地
  // 消失，但 <svg> 還在、測試照樣全綠。這兩個測試就是釘住這件事。
  // 以下幾條釘住 dataviz 規範裡「可驗證」的 mark 規格，避免日後被順手改回去。
  // 規範：格線是 hairline 實線、絕不用虛線；bar chart 只留水平格線。
  it("draws solid horizontal-only gridlines, never dashed", () => {
    const { container } = render(<ArtifactChart spec={barSpec} />);
    const grid = container.querySelector(".recharts-cartesian-grid")!;
    for (const line of Array.from(grid.querySelectorAll("line"))) {
      expect(line.getAttribute("stroke-dasharray")).toBeNull();
    }
    expect(grid.querySelector(".recharts-cartesian-grid-vertical")).toBeNull();
  });

  // 規範：座標軸要 recessive——軸線與刻度線都不畫，只留文字。
  it("keeps the axes recessive: no axis lines, no tick lines", () => {
    const { container } = render(<ArtifactChart spec={barSpec} />);
    expect(container.querySelector(".recharts-cartesian-axis-line")).toBeNull();
    expect(container.querySelector(".recharts-cartesian-axis-tick-line")).toBeNull();
  });

  // 規範：Y 軸刻度要 thousands-comma'd——它承載了沒有被直接標註的數值。
  it("formats y-axis ticks with thousand separators", () => {
    const bigSpec: ChartSpec = {
      ...barSpec,
      data: [{ month: "Jan", sales: 12000 }, { month: "Feb", sales: 4000 }],
    };
    const { container } = render(<ArtifactChart spec={bigSpec} />);
    expect(container.textContent).toContain("12,000");
  });

  it("bar chart really renders the grid, both axes and one rect per data point", () => {
    const { container } = render(<ArtifactChart spec={barSpec} />);
    expect(container.querySelector(".recharts-cartesian-grid")).not.toBeNull();
    expect(container.querySelectorAll(".recharts-cartesian-axis").length).toBeGreaterThanOrEqual(2);
    expect(container.querySelectorAll(".recharts-bar-rectangle")).toHaveLength(barSpec.data.length);
  });

  it("line chart really renders the grid, both axes and the line curve", () => {
    const { container } = render(<ArtifactChart spec={{ ...barSpec, type: "line" }} />);
    expect(container.querySelector(".recharts-cartesian-grid")).not.toBeNull();
    expect(container.querySelectorAll(".recharts-cartesian-axis").length).toBeGreaterThanOrEqual(2);
    expect(container.querySelector(".recharts-line-curve")).not.toBeNull();
  });
});
