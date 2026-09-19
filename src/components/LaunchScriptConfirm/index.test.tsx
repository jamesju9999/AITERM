import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LaunchScriptConfirm } from ".";
import { LocaleProvider } from "../../contexts/LocaleContext";

function mount(props: Partial<React.ComponentProps<typeof LaunchScriptConfirm>> = {}) {
  const onRun = vi.fn();
  const onSkip = vi.fn();
  render(
    <LocaleProvider>
      <LaunchScriptConfirm scriptPath="/proj/deploy.command" onRun={onRun} onSkip={onSkip} {...props} />
    </LocaleProvider>,
  );
  return { onRun, onSkip };
}

// 對話框出現後有一小段時間不接受「執行」：見 LaunchScriptConfirm 的註解。
const RUN_SHIELD_MS = 600;
// 只凍結 Date 與 performance、不假造 setTimeout：RTL 的 asyncWrapper／userEvent 的
// 內部延遲要用真的計時器，整組假計時器會讓它們永遠等不到（測試逾時）。
// 時間用 advanceTimersByTime 往前撥——vi.setSystemTime 只動 Date，不動 performance.now()。
const freezeClock = () => vi.useFakeTimers({ toFake: ["Date", "performance"] });
const advanceClock = (ms: number) => vi.advanceTimersByTime(ms);

afterEach(() => {
  vi.useRealTimers();
});

describe("LaunchScriptConfirm", () => {
  it("shows the full script path so the user knows exactly what would run", () => {
    mount({ scriptPath: "/proj/my folder/deploy.command" });
    expect(screen.getByRole("dialog")).toHaveTextContent("/proj/my folder/deploy.command");
  });

  it("Run and Skip call their own handlers and nothing else", async () => {
    freezeClock();
    const { onRun, onSkip } = mount();
    advanceClock(RUN_SHIELD_MS);
    await userEvent.click(screen.getByTestId("launch-script-run"));
    expect(onRun).toHaveBeenCalledTimes(1);
    expect(onSkip).not.toHaveBeenCalled();
    await userEvent.click(screen.getByTestId("launch-script-skip"));
    expect(onSkip).toHaveBeenCalledTimes(1);
    expect(onRun).toHaveBeenCalledTimes(1);
  });

  it("ignores a Run click that lands right after the dialog appears (double-click / click already in flight)", async () => {
    freezeClock();
    const { onRun } = mount();
    await userEvent.click(screen.getByTestId("launch-script-run"));
    expect(onRun).not.toHaveBeenCalled();
    // 差 1ms 也還在保護期內。
    advanceClock(RUN_SHIELD_MS - 1);
    await userEvent.click(screen.getByTestId("launch-script-run"));
    expect(onRun).not.toHaveBeenCalled();
  });

  it("accepts Run once the shield period has passed", async () => {
    freezeClock();
    const { onRun } = mount();
    advanceClock(RUN_SHIELD_MS);
    await userEvent.click(screen.getByTestId("launch-script-run"));
    expect(onRun).toHaveBeenCalledTimes(1);
  });

  it("uses a monotonic clock: stepping the system clock backwards cannot lock Run out", async () => {
    freezeClock();
    const { onRun } = mount();
    advanceClock(RUN_SHIELD_MS);
    // NTP 校時／VM 休眠還原之類的把系統時間往回撥：Date.now() 會比掛載時還早，
    // 若保護期用 Date 算，「執行」會一直被忽略到系統時間追回來為止。
    vi.setSystemTime(Date.now() - 60 * 60 * 1000);
    await userEvent.click(screen.getByTestId("launch-script-run"));
    expect(onRun).toHaveBeenCalledTimes(1);
  });

  it("never shields Skip: it is the safe path", async () => {
    freezeClock();
    const { onRun, onSkip } = mount();
    await userEvent.click(screen.getByTestId("launch-script-skip"));
    expect(onSkip).toHaveBeenCalledTimes(1);
    expect(onRun).not.toHaveBeenCalled();
  });

  it("puts initial focus on the safe choice, not on Run", () => {
    mount();
    expect(screen.getByTestId("launch-script-skip")).toHaveFocus();
    expect(screen.getByTestId("launch-script-run")).not.toHaveFocus();
  });

  it("a stray Enter (user was typing when the prompt appeared) skips instead of running", async () => {
    const { onRun, onSkip } = mount();
    await userEvent.keyboard("{Enter}");
    expect(onSkip).toHaveBeenCalledTimes(1);
    expect(onRun).not.toHaveBeenCalled();
  });

  it("Escape skips and never runs", async () => {
    const { onRun, onSkip } = mount();
    await userEvent.keyboard("{Escape}");
    expect(onSkip).toHaveBeenCalledTimes(1);
    expect(onRun).not.toHaveBeenCalled();
  });
});
