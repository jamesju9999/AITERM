import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, fireEvent, render, renderHook, screen } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { SidePane } from "./SidePane";
import { clampSide, useSidePane, SIDE_DEFAULT, SIDE_MAX, SIDE_MIN } from "./useSidePane";

beforeEach(() => localStorage.clear());

describe("clampSide", () => {
  it("夾在最小與最大之間", () => {
    expect(clampSide(10, 2000)).toBe(SIDE_MIN);
    expect(clampSide(5000, 2000)).toBe(SIDE_MAX);
    expect(clampSide(400, 2000)).toBe(400);
  });
  it("容器很窄時最大值跟著縮小，畫布至少留 320px", () => {
    expect(clampSide(700, 800)).toBe(480);
  });
  it("容器窄到連最小值都放不下時仍回最小值", () => {
    expect(clampSide(700, 500)).toBe(SIDE_MIN);
  });
});

describe("useSidePane", () => {
  it("預設展開、寬度預設", () => {
    const { result } = renderHook(() => useSidePane());
    expect(result.current.open).toBe(true);
    expect(result.current.width).toBe(SIDE_DEFAULT);
  });
  it("寬度與收合狀態會存起來，下次還原", () => {
    const a = renderHook(() => useSidePane());
    act(() => a.result.current.setWidth(450));
    act(() => a.result.current.toggle());
    const b = renderHook(() => useSidePane());
    expect(b.result.current.width).toBe(450);
    expect(b.result.current.open).toBe(false);
  });
  it("儲存的值壞掉就用預設，localStorage 丟例外也不壞", () => {
    localStorage.setItem("aiterm_arch_side_w", "abc");
    expect(renderHook(() => useSidePane()).result.current.width).toBe(SIDE_DEFAULT);
    const spy = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("x"); });
    expect(renderHook(() => useSidePane()).result.current.open).toBe(true);
    spy.mockRestore();
  });
  it("expand 只展開，不會把已展開的收起來", () => {
    const { result } = renderHook(() => useSidePane());
    act(() => result.current.expand());
    expect(result.current.open).toBe(true);
    act(() => result.current.toggle());
    act(() => result.current.expand());
    expect(result.current.open).toBe(true);
  });
});

function setup(over: Partial<React.ComponentProps<typeof SidePane>> = {}) {
  const props = { open: true, width: 340, onWidth: vi.fn(), onToggle: vi.fn(), children: <div>內容</div>, ...over };
  render(<LocaleProvider><SidePane {...props} /></LocaleProvider>);
  return props;
}

describe("SidePane", () => {
  it("展開時顯示內容與寬度，分隔條是 separator", () => {
    setup({ width: 400 });
    expect(screen.getByText("內容")).toBeInTheDocument();
    expect(screen.getByRole("separator")).toHaveAttribute("aria-valuenow", "400");
    expect((screen.getByTestId("arch-side") as HTMLElement).style.width).toBe("400px");
  });
  it("收合時不渲染內容，只剩展開鈕", () => {
    const p = setup({ open: false });
    expect(screen.queryByText("內容")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /展開側欄|Expand/ }));
    expect(p.onToggle).toHaveBeenCalled();
  });
  it("按收合鈕呼叫 onToggle", () => {
    const p = setup();
    fireEvent.click(screen.getByRole("button", { name: /收合側欄|Collapse/ }));
    expect(p.onToggle).toHaveBeenCalledTimes(1);
  });
  it("鍵盤左右鍵調整寬度（左變寬、右變窄）", () => {
    const p = setup({ width: 340 });
    const sep = screen.getByRole("separator");
    fireEvent.keyDown(sep, { key: "ArrowLeft" });
    expect(p.onWidth).toHaveBeenLastCalledWith(340 + 24);
    fireEvent.keyDown(sep, { key: "ArrowRight" });
    expect(p.onWidth).toHaveBeenLastCalledWith(340 - 24);
  });
  it("雙擊分隔條還原預設寬度", () => {
    const p = setup({ width: 500 });
    fireEvent.doubleClick(screen.getByRole("separator"));
    expect(p.onWidth).toHaveBeenCalledWith(SIDE_DEFAULT);
  });
  it("拖曳：往左拖變寬", () => {
    const p = setup({ width: 340 });
    const sep = screen.getByRole("separator");
    sep.setPointerCapture = vi.fn();
    sep.releasePointerCapture = vi.fn();
    fireEvent.pointerDown(sep, { clientX: 1000, pointerId: 1 });
    fireEvent.pointerMove(sep, { clientX: 900, pointerId: 1 });
    expect(p.onWidth).toHaveBeenLastCalledWith(440);
    fireEvent.pointerUp(sep, { pointerId: 1 });
    fireEvent.pointerMove(sep, { clientX: 700, pointerId: 1 });
    expect(p.onWidth).toHaveBeenCalledTimes(1);
  });
});
