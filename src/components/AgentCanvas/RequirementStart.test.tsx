import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../lib/webviewFocus", () => ({ restoreWebviewFocus: vi.fn().mockResolvedValue(undefined) }));
vi.mock("../../ipc/ai", () => ({ invokeAiComplete: vi.fn() }));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn() }));
vi.mock("../../ipc/tasks", () => ({
  createTask: vi.fn().mockResolvedValue("task-1"),
  getProjectBrief: vi.fn().mockResolvedValue(""),
  setProjectBrief: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../../ipc/agents", () => ({
  assignAgent: vi.fn().mockResolvedValue(undefined),
  checkRepo: vi.fn().mockResolvedValue({ status: "ok", branch: "main" }),
  createAgent: vi.fn().mockResolvedValue("agent-1"),
  getReferences: vi.fn().mockResolvedValue([]),
  setProjectRepo: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../TaskBoard/useRefineTask", () => ({
  useRefineTask: () => ({ busy: false, error: null, refine: vi.fn() }),
}));

import { invokeAiComplete } from "../../ipc/ai";
import { checkRepo } from "../../ipc/agents";
import { createTask } from "../../ipc/tasks";
import { listProviders } from "../../ipc/provider";
import { RequirementStart } from "./RequirementStart";

const provider = { id: "p1", name: "P", is_default: true } as never;

const decisionReply =
  "建議批次。\n```decision\n" +
  JSON.stringify({ approach: "batch", reason: "骨架相同", trial: "先試 5 支", requirement: "轉換 Cm 畫面" }) +
  "\n```";

function renderStart(onStarted = vi.fn()) {
  render(
    <LocaleProvider>
      <RequirementStart projectId="proj" repo="/repo" onRepoChange={vi.fn()} onStarted={onStarted} />
    </LocaleProvider>,
  );
  return onStarted;
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(listProviders).mockResolvedValue([provider]);
  vi.mocked(checkRepo).mockResolvedValue({ status: "ok", branch: "main" });
});

describe("RequirementStart method discussion", () => {
  it("hides the toggle when no AI model is configured", async () => {
    vi.mocked(listProviders).mockResolvedValue([]);
    renderStart();
    await screen.findByTestId("ac-req");
    await waitFor(() => expect(screen.queryByTestId("ac-md-toggle")).toBeNull());
  });

  it("swaps the textarea for the discussion when toggled, and back", async () => {
    renderStart();
    fireEvent.click(await screen.findByTestId("ac-md-toggle"));
    expect(screen.getByTestId("ac-md")).toBeTruthy();
    expect(screen.queryByTestId("ac-req-input")).toBeNull();
    fireEvent.click(screen.getByTestId("ac-md-back"));
    expect(screen.getByTestId("ac-req-input")).toBeTruthy();
  });

  it("confirming a decision builds the team with that decision in the card", async () => {
    // 第一次呼叫是討論回覆；之後（團隊推薦、完成定義）都失敗 → 走預設團隊、沒有完成定義。
    vi.mocked(invokeAiComplete)
      .mockResolvedValueOnce({ content: decisionReply } as never)
      .mockRejectedValue(new Error("no ai"));
    const onStarted = renderStart();
    fireEvent.click(await screen.findByTestId("ac-md-toggle"));
    fireEvent.change(screen.getByTestId("ac-md-input"), { target: { value: "533 支畫面" } });
    fireEvent.click(screen.getByTestId("ac-md-send"));
    await screen.findByTestId("ac-md-decision");
    fireEvent.click(screen.getByTestId("ac-md-confirm"));
    await waitFor(() => expect(onStarted).toHaveBeenCalledTimes(1));
    const card = vi.mocked(createTask).mock.calls[0][1];
    expect(card.title).toContain("轉換 Cm 畫面");
    expect(card.body).toContain("做法決定：");
    expect(card.body).toContain("試跑計畫：先試 5 支");
  });

  it("locks the confirm button while the repo check is not ok", async () => {
    vi.mocked(checkRepo).mockImplementation(async (path) => (path === "/bad" ? { status: "missing" } : { status: "ok", branch: "main" }));
    vi.mocked(invokeAiComplete).mockResolvedValueOnce({ content: decisionReply } as never);
    const props = { projectId: "proj", onRepoChange: vi.fn(), onStarted: vi.fn() };
    const ui = (repo: string) => (
      <LocaleProvider>
        <RequirementStart {...props} repo={repo} />
      </LocaleProvider>
    );
    const { rerender } = render(ui("/repo"));
    fireEvent.click(await screen.findByTestId("ac-md-toggle"));
    fireEvent.change(screen.getByTestId("ac-md-input"), { target: { value: "533 支畫面" } });
    await waitFor(() => expect((screen.getByTestId("ac-md-send") as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(screen.getByTestId("ac-md-send"));
    await screen.findByTestId("ac-md-decision");
    await waitFor(() => expect((screen.getByTestId("ac-md-confirm") as HTMLButtonElement).disabled).toBe(false));
    rerender(ui("/bad"));
    await waitFor(() => expect((screen.getByTestId("ac-md-confirm") as HTMLButtonElement).disabled).toBe(true));
  });
});
