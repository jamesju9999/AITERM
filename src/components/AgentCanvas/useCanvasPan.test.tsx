import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { useRef } from "react";

import { useCanvasPan } from "./useCanvasPan";

/** 跟桌布同樣的結構：捲動區（section）→ sizer → world → 節點／按鈕。 */
function Harness() {
  const ref = useRef<HTMLElement>(null);
  const onMouseDown = useCanvasPan(ref);
  return (
    <section ref={ref} data-testid="canvas" onMouseDown={onMouseDown}>
      <div className="ac-sizer" data-testid="sizer">
        <div className="ac-world" data-testid="world">
          <div data-agent-id="a1" data-testid="node">
            <button data-testid="btn">補充</button>
          </div>
        </div>
      </div>
    </section>
  );
}

const setup = () => {
  render(<Harness />);
  const canvas = screen.getByTestId("canvas");
  canvas.scrollLeft = 100;
  canvas.scrollTop = 80;
  return canvas;
};

describe("桌布滑鼠拖曳平移", () => {
  it("在空白處按住拖曳，桌布跟著手移動（內容往拖的方向走＝捲動位置反向）", () => {
    const canvas = setup();
    fireEvent.mouseDown(screen.getByTestId("world"), { button: 0, clientX: 300, clientY: 300 });
    fireEvent.mouseMove(window, { clientX: 270, clientY: 280 });
    expect(canvas.scrollLeft).toBe(130);
    expect(canvas.scrollTop).toBe(100);
    fireEvent.mouseMove(window, { clientX: 320, clientY: 310 });
    expect(canvas.scrollLeft).toBe(80);
    expect(canvas.scrollTop).toBe(70);
    fireEvent.mouseUp(window);
  });

  it("桌布本身、sizer、world 的空白處都能起手", () => {
    for (const id of ["canvas", "sizer", "world"]) {
      const canvas = setup();
      fireEvent.mouseDown(screen.getByTestId(id), { button: 0, clientX: 10, clientY: 10 });
      fireEvent.mouseMove(window, { clientX: 0, clientY: 0 });
      expect(canvas.scrollLeft, id).toBe(110);
      fireEvent.mouseUp(window);
      document.body.innerHTML = "";
    }
  });

  it("點在節點或按鈕上不平移（那是拖節點、按按鈕）", () => {
    const canvas = setup();
    for (const id of ["node", "btn"]) {
      fireEvent.mouseDown(screen.getByTestId(id), { button: 0, clientX: 300, clientY: 300 });
      fireEvent.mouseMove(window, { clientX: 200, clientY: 200 });
      fireEvent.mouseUp(window);
    }
    expect(canvas.scrollLeft).toBe(100);
    expect(canvas.scrollTop).toBe(80);
  });

  it("只有左鍵；放開之後再移動不會繼續捲", () => {
    const canvas = setup();
    fireEvent.mouseDown(screen.getByTestId("world"), { button: 2, clientX: 300, clientY: 300 });
    fireEvent.mouseMove(window, { clientX: 200, clientY: 200 });
    expect(canvas.scrollLeft).toBe(100);

    fireEvent.mouseDown(screen.getByTestId("world"), { button: 0, clientX: 300, clientY: 300 });
    fireEvent.mouseMove(window, { clientX: 290, clientY: 300 });
    fireEvent.mouseUp(window);
    fireEvent.mouseMove(window, { clientX: 100, clientY: 100 });
    expect(canvas.scrollLeft).toBe(110);
  });

  it("拖曳期間桌布有 panning 樣式（游標變抓手），放開就拿掉", () => {
    const canvas = setup();
    expect(canvas.classList.contains("ac-canvas--panning")).toBe(false);
    fireEvent.mouseDown(screen.getByTestId("world"), { button: 0, clientX: 5, clientY: 5 });
    expect(canvas.classList.contains("ac-canvas--panning")).toBe(true);
    fireEvent.mouseUp(window);
    expect(canvas.classList.contains("ac-canvas--panning")).toBe(false);
  });
});
