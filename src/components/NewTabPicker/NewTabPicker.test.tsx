import { describe, it, expect } from "vitest";
import { Tab } from "../TabBar";

describe("Tab type", () => {
  it("accepts terminal type", () => {
    const tab: Tab = { id: "1", title: "Terminal", type: "terminal" };
    expect(tab.type).toBe("terminal");
  });

  it("accepts database type with connection id", () => {
    const tab: Tab = { id: "2", title: "DB", type: "database", dbConnectionId: "conn-1" };
    expect(tab.type).toBe("database");
    expect(tab.dbConnectionId).toBe("conn-1");
  });
});
