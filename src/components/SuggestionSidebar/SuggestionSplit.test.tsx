import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { SuggestionSplit, SIDEBAR_WIDTH_KEY, MIN_MAIN_WIDTH, MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH } from "./SuggestionSplit";

// 讓測試能指定容器寬度：jsdom 沒有版面，ResizeObserver 也不存在。
let roCallback: ((entries: { contentRect: { width: number } }[]) => void) | null = null;
class FakeResizeObserver {
  constructor(cb: (entries: { contentRect: { width: number } }[]) => void) { roCallback = cb; }
  observe() {}
  disconnect() {}
  unobserve() {}
}
const setContainerWidth = (w: number) => act(() => { roCallback?.([{ contentRect: { width: w } }]); });

beforeEach(() => {
  localStorage.clear();
  roCallback = null;
  vi.stubGlobal("ResizeObserver", FakeResizeObserver);
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue(
    { left: 0, right: 1200, width: 1200, top: 0, bottom: 600, height: 600, x: 0, y: 0, toJSON: () => ({}) } as DOMRect,
  );
});
afterEach(() => { vi.unstubAllGlobals(); vi.restoreAllMocks(); });

const sidebarEl = (c: HTMLElement) => c.querySelector(".aiterm-sugg-split__side") as HTMLElement | null;
const divider = (c: HTMLElement) => c.querySelector(".aiterm-sugg-split__divider") as HTMLElement;
const ptr = (type: string, clientX: number) => new PointerEvent(type, { bubbles: true, pointerId: 1, clientX });

describe("SuggestionSplit", () => {
  it("renders only the main content when there is no sidebar", () => {
    const { container } = render(<SuggestionSplit sidebar={null}><div>TERM</div></SuggestionSplit>);
    expect(screen.getByText("TERM")).toBeInTheDocument();
    expect(sidebarEl(container)).toBeNull();
    expect(container.querySelector(".aiterm-sugg-split__divider")).toBeNull();
  });

  it("puts the sidebar next to the terminal (side by side), not over it", () => {
    const { container } = render(<SuggestionSplit sidebar={<div>SIDE</div>}><div>TERM</div></SuggestionSplit>);
    setContainerWidth(1200);
    expect(screen.getByText("SIDE")).toBeInTheDocument();
    const main = screen.getByText("TERM").parentElement as HTMLElement;
    // 主欄是可收縮的彈性欄；側欄是固定寬度——兩者都在同一個 flex 列裡，沒有 absolute。
    expect(main.style.flex).toContain("1");
    expect(main.style.minWidth).toBe("0px");
    expect(sidebarEl(container)!.style.width).toBe("340px");
    expect(sidebarEl(container)!.style.position).not.toBe("absolute");
    expect(divider(container).getAttribute("role")).toBe("separator");
  });

  it("drags with pointer capture, leaves the layout alone while dragging, and commits once on release", () => {
    const { container } = render(<SuggestionSplit sidebar={<div>SIDE</div>}><div>TERM</div></SuggestionSplit>);
    setContainerWidth(1200);
    const grip = divider(container);
    let captured = false;
    grip.setPointerCapture = () => { captured = true; };
    act(() => { grip.dispatchEvent(ptr("pointerdown", 860)); });
    expect(captured).toBe(true);

    act(() => { grip.dispatchEvent(ptr("pointermove", 800)); });
    act(() => { grip.dispatchEvent(ptr("pointermove", 700)); });
    // 拖曳中側欄寬度（也就是終端機寬度）完全不變：否則每個 pointermove 都會觸發一次 PTY resize。
    expect(sidebarEl(container)!.style.width).toBe("340px");
    expect(container.querySelector(".aiterm-sugg-split__ghost")).not.toBeNull();

    act(() => { grip.dispatchEvent(ptr("pointerup", 700)); });
    expect(sidebarEl(container)!.style.width).toBe("500px"); // 1200 - 700
    expect(container.querySelector(".aiterm-sugg-split__ghost")).toBeNull();
    expect(localStorage.getItem(SIDEBAR_WIDTH_KEY)).toBe("500");
  });

  it("clamps the dragged width to the allowed range", () => {
    const { container } = render(<SuggestionSplit sidebar={<div>SIDE</div>}><div>TERM</div></SuggestionSplit>);
    setContainerWidth(1200);
    const grip = divider(container);
    grip.setPointerCapture = () => {};
    act(() => { grip.dispatchEvent(ptr("pointerdown", 860)); });
    act(() => { grip.dispatchEvent(ptr("pointerup", 1190)); }); // 想拖到只剩 10px
    expect(sidebarEl(container)!.style.width).toBe(`${MIN_SIDEBAR_WIDTH}px`);
    act(() => { grip.dispatchEvent(ptr("pointerdown", 900)); });
    act(() => { grip.dispatchEvent(ptr("pointerup", 100)); }); // 想拖到 1100px
    expect(sidebarEl(container)!.style.width).toBe(`${MAX_SIDEBAR_WIDTH}px`);
  });

  it("restores a saved width, and ignores garbage in storage", () => {
    localStorage.setItem(SIDEBAR_WIDTH_KEY, "420");
    const a = render(<SuggestionSplit sidebar={<div>SIDE</div>}><div>TERM</div></SuggestionSplit>);
    setContainerWidth(1200);
    expect(sidebarEl(a.container)!.style.width).toBe("420px");
    a.unmount();
    localStorage.setItem(SIDEBAR_WIDTH_KEY, "abc");
    const b = render(<SuggestionSplit sidebar={<div>SIDE</div>}><div>TERM</div></SuggestionSplit>);
    setContainerWidth(1200);
    expect(sidebarEl(b.container)!.style.width).toBe("340px");
  });

  it("never lets the terminal column shrink below its minimum", () => {
    localStorage.setItem(SIDEBAR_WIDTH_KEY, "560");
    const { container } = render(<SuggestionSplit sidebar={<div>SIDE</div>}><div>TERM</div></SuggestionSplit>);
    const total = MIN_MAIN_WIDTH + 400;
    setContainerWidth(total);
    const w = parseInt(sidebarEl(container)!.style.width, 10);
    expect(total - w).toBeGreaterThanOrEqual(MIN_MAIN_WIDTH);
  });

  it("hides the sidebar when the window is too narrow for both columns", () => {
    const { container } = render(<SuggestionSplit sidebar={<div>SIDE</div>}><div>TERM</div></SuggestionSplit>);
    setContainerWidth(MIN_MAIN_WIDTH + MIN_SIDEBAR_WIDTH - 1);
    expect(screen.queryByText("SIDE")).toBeNull();
    expect(screen.getByText("TERM")).toBeInTheDocument();
    expect(divider(container)).toBeNull();
    setContainerWidth(MIN_MAIN_WIDTH + MIN_SIDEBAR_WIDTH + 20);
    expect(screen.getByText("SIDE")).toBeInTheDocument();
  });
});
