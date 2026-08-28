import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AgentPanel } from "./AgentPanel";
import { INITIAL_PREVIEW } from "../../lib/agentLoop";
import { translations } from "../../lib/i18n";

const t = translations["en"];
const baseProps = {
  mission: null,
  phase: null,
  streamText: "",
  preview: INITIAL_PREVIEW,
  onSubmitGoal: vi.fn(),
  onStop: vi.fn(),
  onConfirmPreview: vi.fn(),
  onCancelPreview: vi.fn(),
  onClose: vi.fn(),
  disabled: false,
  t,
};

describe("AgentPanel", () => {
  it("submits a typed goal via onSubmitGoal", async () => {
    const onSubmitGoal = vi.fn();
    render(<AgentPanel {...baseProps} onSubmitGoal={onSubmitGoal} />);
    await userEvent.type(screen.getByRole("textbox"), "find big files");
    await userEvent.keyboard("{Enter}");
    expect(onSubmitGoal).toHaveBeenCalledWith("find big files");
  });

  it("renders one card per mission history step with exit-code badge", () => {
    render(
      <AgentPanel
        {...baseProps}
        mission={{
          goal: "g", active: true, stepCount: 2, maxSteps: 5, tokensUsed: 0,
          history: [
            { command: "pwd", exitCode: 0, output: "/home/u" },
            { command: "false", exitCode: 1, output: "" },
          ],
        }}
      />
    );
    expect(screen.getByText("pwd")).toBeInTheDocument();
    expect(screen.getByText("false")).toBeInTheDocument();
    expect(screen.getByText("exit 1")).toBeInTheDocument();
  });

  it("shows a Stop button while the mission is active and calls onStop", async () => {
    const onStop = vi.fn();
    render(
      <AgentPanel
        {...baseProps}
        onStop={onStop}
        mission={{ goal: "g", active: true, stepCount: 0, maxSteps: 5, tokensUsed: 0, history: [] }}
      />
    );
    await userEvent.click(screen.getByRole("button", { name: t.remote_agent_stop }));
    expect(onStop).toHaveBeenCalled();
  });

  it("renders CommandPreview when preview.visible and wires confirm/cancel", async () => {
    const onConfirmPreview = vi.fn();
    render(
      <AgentPanel
        {...baseProps}
        onConfirmPreview={onConfirmPreview}
        preview={{ loading: false, visible: true, command: "rm -rf x", explanation: "danger", riskLevel: "dangerous" }}
      />
    );
    expect(screen.getByText("rm -rf x")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /Execute Anyway/i }));
    expect(onConfirmPreview).toHaveBeenCalled();
  });

  it("shows the failed reason when phase is failed", () => {
    render(<AgentPanel {...baseProps} phase={{ phase: "failed", reason: "lost control" }} />);
    expect(screen.getByText(/lost control/)).toBeInTheDocument();
  });
});
