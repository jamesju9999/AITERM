// src/components/TaskBoard/transcriptUpgrade.test.ts
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("../../lib/terminalInstanceRegistry", () => ({
  serializeTerminal: vi.fn(),
}));
vi.mock("../../ipc/tasks", () => ({
  saveTranscript: vi.fn().mockResolvedValue(undefined),
}));

import { serializeTerminal } from "../../lib/terminalInstanceRegistry";
import { saveTranscript } from "../../ipc/tasks";
import { tryUpgradeTranscript } from "./transcriptUpgrade";

beforeEach(() => {
  vi.mocked(serializeTerminal).mockReset();
  vi.mocked(saveTranscript).mockReset().mockResolvedValue(undefined);
});

describe("tryUpgradeTranscript", () => {
  it("does nothing when tabId is null", async () => {
    await tryUpgradeTranscript("task-1", null);
    expect(serializeTerminal).not.toHaveBeenCalled();
    expect(saveTranscript).not.toHaveBeenCalled();
  });

  it("does nothing when the tab is not registered (serializeTerminal returns null)", async () => {
    vi.mocked(serializeTerminal).mockReturnValue(null);
    await tryUpgradeTranscript("task-1", "tab-1");
    expect(saveTranscript).not.toHaveBeenCalled();
  });

  it("strips ANSI codes and saves when the tab is live", async () => {
    vi.mocked(serializeTerminal).mockReturnValue("\x1b[32mhello\x1b[0m world");
    await tryUpgradeTranscript("task-1", "tab-1");
    expect(saveTranscript).toHaveBeenCalledWith("task-1", "hello world");
  });

  it("swallows a saveTranscript failure instead of throwing", async () => {
    vi.mocked(serializeTerminal).mockReturnValue("clean");
    vi.mocked(saveTranscript).mockRejectedValue(new Error("disk full"));
    await expect(tryUpgradeTranscript("task-1", "tab-1")).resolves.toBeUndefined();
  });
});
