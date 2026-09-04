import { describe, expect, it } from "vitest";
import { isRunningTaskTab, setRunningTaskTabs } from "./runningTaskTabRegistry";

describe("runningTaskTabRegistry", () => {
  it("returns false for a tab id that was never registered", () => {
    expect(isRunningTaskTab("never-registered")).toBe(false);
  });

  it("returns true for a tab id passed to setRunningTaskTabs", () => {
    setRunningTaskTabs(["tab-1", "tab-2"]);
    expect(isRunningTaskTab("tab-1")).toBe(true);
    expect(isRunningTaskTab("tab-2")).toBe(true);
    expect(isRunningTaskTab("tab-3")).toBe(false);
  });

  it("fully replaces the previous set — a tab id dropped from the new list stops being reported as running", () => {
    setRunningTaskTabs(["tab-1", "tab-2"]);
    setRunningTaskTabs(["tab-2"]);
    expect(isRunningTaskTab("tab-1")).toBe(false);
    expect(isRunningTaskTab("tab-2")).toBe(true);
  });

  it("an empty list clears everything", () => {
    setRunningTaskTabs(["tab-1"]);
    setRunningTaskTabs([]);
    expect(isRunningTaskTab("tab-1")).toBe(false);
  });
});
