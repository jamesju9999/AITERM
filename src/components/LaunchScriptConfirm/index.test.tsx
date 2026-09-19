import { describe, it, expect, vi } from "vitest";
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

describe("LaunchScriptConfirm", () => {
  it("shows the full script path so the user knows exactly what would run", () => {
    mount({ scriptPath: "/proj/my folder/deploy.command" });
    expect(screen.getByRole("dialog")).toHaveTextContent("/proj/my folder/deploy.command");
  });

  it("Run and Skip call their own handlers and nothing else", async () => {
    const { onRun, onSkip } = mount();
    await userEvent.click(screen.getByTestId("launch-script-run"));
    expect(onRun).toHaveBeenCalledTimes(1);
    expect(onSkip).not.toHaveBeenCalled();
    await userEvent.click(screen.getByTestId("launch-script-skip"));
    expect(onSkip).toHaveBeenCalledTimes(1);
    expect(onRun).toHaveBeenCalledTimes(1);
  });
});
