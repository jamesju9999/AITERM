import { render, screen, fireEvent, act } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { TourBar } from "./TourBar";
import { autoplayDelayMs } from "./tour";
import type { Tour } from "./qa";

const TOUR: Tour = {
  title: "登入流程",
  steps: [
    { module: "a", caption: "第一步說明" },
    { module: "b", caption: "第二步說明", evidence: { file: "x/y.ts", line: 7 } },
    { module: "c", caption: "第三步說明" },
  ],
};

function Harness({ playing0, onClose = vi.fn(), onOpenFile = vi.fn(), onStep }: { playing0: boolean; onClose?: () => void; onOpenFile?: (p: string, l?: number) => void; onStep?: (i: number) => void }) {
  const [index, setIndex] = useState(0);
  const [playing, setPlaying] = useState(playing0);
  return (
    <LocaleProvider>
      <TourBar tour={TOUR} index={index} playing={playing}
        onIndex={(i) => { setIndex(i); onStep?.(i); }} onPlaying={setPlaying} onClose={onClose} onOpenFile={onOpenFile} />
      <input aria-label="other" />
    </LocaleProvider>
  );
}
const mount = async (el: React.ReactElement) => { const r = render(el); await act(async () => {}); return r; };
const ctl = (re: RegExp) => screen.getByRole("button", { name: re });

afterEach(() => vi.useRealTimers());

describe("TourBar", () => {
  it("顯示第 i/n 步、標題與說明；上一步在第一步停用，下一步在最後一步停用", async () => {
    await mount(<Harness playing0={false} />);
    expect(screen.getByText(/第 1\/3 步|Step 1\/3/)).toBeInTheDocument();
    expect(screen.getByText("登入流程")).toBeInTheDocument();
    expect(screen.getByText("第一步說明")).toBeInTheDocument();
    expect(ctl(/上一步|Previous step/)).toBeDisabled();
    fireEvent.click(ctl(/下一步|Next step/));
    expect(screen.getByText("第二步說明")).toBeInTheDocument();
    fireEvent.click(ctl(/下一步|Next step/));
    expect(ctl(/下一步|Next step/)).toBeDisabled();
    fireEvent.click(ctl(/上一步|Previous step/));
    expect(screen.getByText(/第 2\/3 步|Step 2\/3/)).toBeInTheDocument();
  });

  it("佐證 file:line 是按鈕，點下去帶檔案與行號；沒有佐證就不顯示", async () => {
    const onOpenFile = vi.fn();
    await mount(<Harness playing0={false} onOpenFile={onOpenFile} />);
    expect(screen.queryByText("x/y.ts:7")).toBeNull();
    fireEvent.click(ctl(/下一步|Next step/));
    fireEvent.click(screen.getByRole("button", { name: "x/y.ts:7" }));
    expect(onOpenFile).toHaveBeenCalledWith("x/y.ts", 7);
  });

  it("播放時每 autoplayDelayMs 前進一步，最後一步停住並變回播放鈕", async () => {
    vi.useFakeTimers();
    await mount(<Harness playing0 />);
    expect(screen.getByText(/第 1\/3 步|Step 1\/3/)).toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(autoplayDelayMs - 1); });
    expect(screen.getByText(/第 1\/3 步|Step 1\/3/)).toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(1); });
    expect(screen.getByText(/第 2\/3 步|Step 2\/3/)).toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(autoplayDelayMs); });
    expect(screen.getByText(/第 3\/3 步|Step 3\/3/)).toBeInTheDocument();
    act(() => { vi.advanceTimersByTime(autoplayDelayMs * 3); });
    expect(screen.getByText(/第 3\/3 步|Step 3\/3/)).toBeInTheDocument();
    expect(ctl(/^(播放|Play)$/)).toBeInTheDocument();
  });

  it("暫停後不再前進；手動換步會重新計時", async () => {
    vi.useFakeTimers();
    await mount(<Harness playing0 />);
    act(() => { vi.advanceTimersByTime(autoplayDelayMs - 100); });
    fireEvent.click(ctl(/下一步|Next step/)); // 換步：計時重來
    act(() => { vi.advanceTimersByTime(autoplayDelayMs - 100); });
    expect(screen.getByText(/第 2\/3 步|Step 2\/3/)).toBeInTheDocument();
    fireEvent.click(ctl(/^(暫停|Pause)$/));
    act(() => { vi.advanceTimersByTime(autoplayDelayMs * 5); });
    expect(screen.getByText(/第 2\/3 步|Step 2\/3/)).toBeInTheDocument();
  });

  it("卸載後計時器被清掉", async () => {
    vi.useFakeTimers();
    const onStep = vi.fn();
    const { unmount } = await mount(<Harness playing0 onStep={onStep} />);
    unmount();
    act(() => { vi.advanceTimersByTime(autoplayDelayMs * 3); });
    expect(onStep).not.toHaveBeenCalled();
  });

  it("Esc 結束導覽；方向鍵換步，但焦點在輸入框時不動", async () => {
    const onClose = vi.fn();
    await mount(<Harness playing0={false} onClose={onClose} />);
    fireEvent.keyDown(document, { key: "ArrowRight" });
    expect(screen.getByText(/第 2\/3 步|Step 2\/3/)).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "ArrowLeft" });
    expect(screen.getByText(/第 1\/3 步|Step 1\/3/)).toBeInTheDocument();
    fireEvent.keyDown(screen.getByLabelText("other"), { key: "ArrowRight" });
    expect(screen.getByText(/第 1\/3 步|Step 1\/3/)).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    fireEvent.click(ctl(/結束導覽|End tour/));
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
