import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../contexts/LocaleContext";
import { AgentStatusBar, type AgentPhase } from "./AgentStatusBar";

function renderBar(status: AgentPhase, onDismiss = vi.fn()) {
  render(
    <LocaleProvider>
      <AgentStatusBar status={status} onDismiss={onDismiss} />
    </LocaleProvider>,
  );
  return onDismiss;
}

describe("AgentStatusBar", () => {
  it("shows the step counter and command for the running phase", () => {
    renderBar({ phase: "running", step: 2, maxSteps: 5, command: "ls -la" });
    expect(screen.getByText(/ls -la/)).toBeInTheDocument();
    expect(screen.getByText("步驟 2/5")).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("shows the query for the web phase", () => {
    renderBar({ phase: "web", step: 1, maxSteps: 5, query: "weather taipei", webKind: "search" });
    expect(screen.getByText(/weather taipei/)).toBeInTheDocument();
  });

  it("renders a dismiss button for the done phase and fires onDismiss", async () => {
    const onDismiss = renderBar({ phase: "done", steps: 3 });
    const btn = screen.getByRole("button", { name: "關閉" });
    await userEvent.click(btn);
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("renders the reason and a dismiss button for the failed phase", () => {
    renderBar({ phase: "failed", reason: "指令逾時" });
    expect(screen.getByText(/指令逾時/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "關閉" })).toBeInTheDocument();
  });

  it("does not show a step counter for done", () => {
    renderBar({ phase: "done", steps: 3 });
    expect(screen.queryByText(/步驟/)).not.toBeInTheDocument();
  });

  it("顯示本次 mission 的累計 token", () => {
    render(
      <LocaleProvider>
        <AgentStatusBar
          status={{ phase: "running", step: 1, maxSteps: 5, command: "ls" }}
          onDismiss={vi.fn()}
          missionTokens={12400}
        />
      </LocaleProvider>,
    );
    expect(screen.getByTestId("mission-tokens")).toHaveTextContent("12.4k");
  });

  it("token 為 0 時不顯示這一段", () => {
    render(
      <LocaleProvider>
        <AgentStatusBar
          status={{ phase: "running", step: 1, maxSteps: 5, command: "ls" }}
          onDismiss={vi.fn()}
          missionTokens={0}
        />
      </LocaleProvider>,
    );
    expect(screen.queryByTestId("mission-tokens")).not.toBeInTheDocument();
  });
});
