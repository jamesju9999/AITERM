import { describe, it, expect, beforeEach } from "vitest";
import { buildChecklist, loadDismissed, saveDismissed, type ChecklistInput } from "./checklistLogic";

const none: ChecklistInput = { providers: 0, mailAccounts: [], mcpServers: 0, dbConnections: 0, vcsConnections: 0, bridgeEnabled: false };
const status = (c: ChecklistInput) => Object.fromEntries(buildChecklist(c).map((i) => [i.id, i.status]));

describe("buildChecklist", () => {
  it("nothing configured → everything todo, and only the provider is required", () => {
    expect(Object.values(status(none)).every((s) => s === "todo")).toBe(true);
    expect(buildChecklist(none).filter((i) => i.required).map((i) => i.id)).toEqual(["provider"]);
  });

  it("each item flips to done on its own signal", () => {
    expect(status({ ...none, providers: 1 }).provider).toBe("done");
    expect(status({ ...none, mcpServers: 2 }).mcp).toBe("done");
    expect(status({ ...none, dbConnections: 1 }).db).toBe("done");
    expect(status({ ...none, vcsConnections: 1 }).vcs).toBe("done");
    expect(status({ ...none, bridgeEnabled: true }).bridge).toBe("done");
  });

  it("mail: a missing enabled field counts as enabled; all-disabled is 'warn', not 'done'", () => {
    expect(status({ ...none, mailAccounts: [{}] }).mail).toBe("done");
    expect(status({ ...none, mailAccounts: [{ enabled: false }] }).mail).toBe("warn");
    expect(status({ ...none, mailAccounts: [{ enabled: false }, { enabled: true }] }).mail).toBe("done");
  });

  it("jump targets match the settings tabs", () => {
    expect(buildChecklist(none).map((i) => i.tab)).toEqual(["providers", "mail", "mcp", "databases", "vcs", "bridge"]);
  });
});

describe("dismissed storage", () => {
  beforeEach(() => localStorage.clear());
  it("round-trips, and tolerates garbage", () => {
    saveDismissed(new Set(["mcp", "db"]));
    expect([...loadDismissed()].sort()).toEqual(["db", "mcp"]);
    localStorage.setItem("aiterm_setup_dismissed", "{not json");
    expect(loadDismissed().size).toBe(0);
    localStorage.setItem("aiterm_setup_dismissed", JSON.stringify([1, "x"]));
    expect([...loadDismissed()]).toEqual(["x"]);
  });
});
