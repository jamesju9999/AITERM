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

  // 上面那些「有沒有畫出 svg」的斷言擋不住一整類靜默失效：recharts 是靠掃描
  // children 找出格線/座標軸/圖例的，實作把這些共用元件放進一個 <>...</>
  // fragment 裡重用，如果 recharts 不會攤平 fragment，格線與座標軸會安靜地
  // 消失，但 <svg> 還在、測試照樣全綠。這兩個測試就是釘住這件事。
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
