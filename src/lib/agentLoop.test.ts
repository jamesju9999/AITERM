import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AiCommandReady } from "../ipc/ai";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";
import { webSearch } from "../ipc/web";
import { runAgentLoop, type AgentLoopParams } from "./agentLoop";

vi.mock("../ipc/web", () => ({
  webSearch: vi.fn(async () => ""),
  webFetch: vi.fn(async () => ""),
}));

// Minimal i18n stub — only the keys runAgentLoop / handleAiQuery actually read.
const t = {
  term_agent_max_steps: (n: number) => `Max steps (${n}) reached`,
  term_agent_timeout: "agent timed out",
  term_agent_timeout_fail: "agent stopped: timeout",
  term_danger_warning: "DANGER",
  term_setup_hint_provider: "",
  term_setup_hint_ollama: "",
  term_setup_hint_api_key: "",
};

const safeResp = (command: string): AiCommandReady => ({
  command,
  explanation: `run ${command}`,
  risk_level: "safe",
});
const doneResp = (): AiCommandReady => ({
  command: "DONE",
  explanation: "all done",
  risk_level: "safe",
});
const fakeBlock = (command: string, rawOutput: string): TerminalBlock => ({
  id: "block-1",
  command,
  status: "completed",
  exitCode: 0,
  startTime: 0,
  endTime: 1,
  rawOutput,
});

/** Drain the microtask queue — the loop is promise-driven, timers stay fake. */
const flush = async (rounds = 20) => {
  for (let i = 0; i < rounds; i++) await Promise.resolve();
};

/** A submitCommand fake that "runs" the command and reports completion at once. */
const instantSubmit = (output = "output") =>
  vi.fn((cmd: string, onComplete?: (block: TerminalBlock) => void) => {
    onComplete?.(fakeBlock(cmd, output));
  });

function makeParams(overrides: Partial<AgentLoopParams> = {}): AgentLoopParams {
  return {
    t,
    goal: "achieve the goal",
    queryFn: vi.fn(async () => doneResp()),
    term: { write: vi.fn() } as unknown as AgentLoopParams["term"],
    getSubmitCommand: () => vi.fn(),
    setPreview: vi.fn(),
    setStreamText: vi.fn(),
    streamingRef: { current: false },
    executionModeRef: { current: "graded" },
    writeRed: vi.fn(),
    abortRef: { current: false },
    stepCount: 0,
    maxSteps: 5,
    history: [],
    onComplete: vi.fn(),
    onFail: vi.fn(),
    ...overrides,
  };
}

describe("runAgentLoop", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
  });
  afterEach(() => {
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  it("recurses across steps, threads history, and completes on DONE", async () => {
    const queryFn = vi
      .fn<(q: string) => Promise<AiCommandReady>>()
      .mockResolvedValueOnce(safeResp("ls -la"))
      .mockResolvedValueOnce(doneResp());
    const submitCommand = instantSubmit("total 8\ndrwxr-xr-x  2 me me");
    const params = makeParams({ queryFn, getSubmitCommand: () => submitCommand });

    runAgentLoop(params);
    await flush();

    expect(queryFn).toHaveBeenCalledTimes(2);
    // Step 2's prompt must carry step 1's command + captured output.
    const secondPrompt = queryFn.mock.calls[1][0];
    expect(secondPrompt).toContain("ls -la");
    expect(secondPrompt).toContain("total 8");
    expect(submitCommand).toHaveBeenCalledTimes(1);
    expect(params.onComplete).toHaveBeenCalledWith("all done");
    expect(params.onFail).not.toHaveBeenCalled();
  });

  it("fails with the max-steps message once the step cap is hit", async () => {
    const queryFn = vi
      .fn<(q: string) => Promise<AiCommandReady>>()
      .mockResolvedValue(safeResp("sleep 1")); // never DONE
    const submitCommand = instantSubmit();
    const params = makeParams({
      queryFn,
      getSubmitCommand: () => submitCommand,
      maxSteps: 1,
    });

    runAgentLoop(params);
    await flush();

    expect(queryFn).toHaveBeenCalledTimes(1);
    expect(params.onFail).toHaveBeenCalledWith(t.term_agent_max_steps(1));
    expect(params.onComplete).not.toHaveBeenCalled();
  });

  it("fails via the 60s timeout when the command never completes", async () => {
    const queryFn = vi
      .fn<(q: string) => Promise<AiCommandReady>>()
      .mockResolvedValue(safeResp("cat")); // waits for stdin forever
    const submitCommand = vi.fn(); // never invokes onComplete
    const params = makeParams({ queryFn, getSubmitCommand: () => submitCommand });

    runAgentLoop(params);
    await flush();

    expect(submitCommand).toHaveBeenCalledTimes(1);
    expect(params.onFail).not.toHaveBeenCalled();

    vi.advanceTimersByTime(60_000);

    expect(params.onFail).toHaveBeenCalledWith(t.term_agent_timeout_fail);
    expect(params.onComplete).not.toHaveBeenCalled();
  });

  it("routes a web-search sentinel through webSearch, not submitCommand", async () => {
    vi.mocked(webSearch).mockResolvedValue("SEARCH RESULT ABOUT FOO");
    const queryFn = vi
      .fn<(q: string) => Promise<AiCommandReady>>()
      .mockResolvedValueOnce(safeResp("AITERM_WEB_SEARCH: foo"))
      .mockResolvedValueOnce(doneResp());
    const submitCommand = instantSubmit();
    const params = makeParams({ queryFn, getSubmitCommand: () => submitCommand });

    runAgentLoop(params);
    await flush();

    expect(webSearch).toHaveBeenCalledWith("foo");
    // The sentinel must never reach the PTY as a shell command.
    expect(submitCommand).not.toHaveBeenCalled();
    expect(queryFn).toHaveBeenCalledTimes(2);
    const secondPrompt = queryFn.mock.calls[1][0];
    expect(secondPrompt).toContain('web_search("foo")');
    expect(secondPrompt).toContain("SEARCH RESULT ABOUT FOO");
    expect(params.onComplete).toHaveBeenCalled();
    expect(params.onFail).not.toHaveBeenCalled();
  });

  it("stops on abort without calling onComplete or onFail", async () => {
    const abortRef = { current: false };
    const queryFn = vi.fn<(q: string) => Promise<AiCommandReady>>(async () => {
      abortRef.current = true; // aborted mid-step, before the next iteration
      return safeResp("echo hi");
    });
    const submitCommand = instantSubmit();
    const params = makeParams({
      abortRef,
      queryFn,
      getSubmitCommand: () => submitCommand,
    });

    runAgentLoop(params);
    await flush();

    expect(queryFn).toHaveBeenCalledTimes(1);
    expect(params.onComplete).not.toHaveBeenCalled();
    expect(params.onFail).not.toHaveBeenCalled();
  });
});
