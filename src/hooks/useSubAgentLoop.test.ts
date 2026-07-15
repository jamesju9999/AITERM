import { describe, expect, it, vi, beforeEach } from "vitest";

const agentChatMock = vi.fn();
vi.mock("../ipc/ai", async () => {
  const actual = await vi.importActual<typeof import("../ipc/ai")>("../ipc/ai");
  return { ...actual, agentChat: (...args: unknown[]) => agentChatMock(...args) };
});
vi.mock("../ipc/fs", () => ({
  readFile: vi.fn(),
  writeTextFile: vi.fn(),
  listDirectory: vi.fn(),
  getSessionCwd: vi.fn().mockResolvedValue("/project"),
}));
vi.mock("../ipc/exec", () => ({ agentExec: vi.fn() }));

import { runSubAgent, type AgentDefinition } from "./useSubAgentLoop";

const agent: AgentDefinition = {
  name: "Coder",
  providerId: "prov1",
  roleDescription: "You write code.",
  tools: [],
};

describe("runSubAgent locale", () => {
  beforeEach(() => {
    agentChatMock.mockReset();
    agentChatMock.mockResolvedValue({ content: "done", tool_calls: [], tool_calling_unsupported: false });
  });

  it("system prompt asks for an English report when locale is en", async () => {
    await runSubAgent("s1", agent, "do the task", { locale: "en" });
    const [, history] = agentChatMock.mock.calls[0];
    const systemMsg = (history as { role: string; content: string }[]).find((m) => m.role === "system");
    expect(systemMsg?.content).toContain("report in English");
  });

  it("system prompt asks for a Traditional Chinese report by default", async () => {
    await runSubAgent("s1", agent, "do the task", {});
    const [, history] = agentChatMock.mock.calls[0];
    const systemMsg = (history as { role: string; content: string }[]).find((m) => m.role === "system");
    expect(systemMsg?.content).toContain("report in Traditional Chinese (繁體中文)");
  });
});
