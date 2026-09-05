import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { listReports, readReport, saveReport } from "./reports";

describe("reports ipc", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue(undefined);
  });

  it("saveReport 傳 projectId 與 html", async () => {
    invoke.mockResolvedValue("2026-09-05-1430.html");
    await expect(saveReport("p1", "<html></html>")).resolves.toBe("2026-09-05-1430.html");
    expect(invoke).toHaveBeenCalledWith("reports_save", { projectId: "p1", html: "<html></html>" });
  });

  it("listReports 傳 projectId", async () => {
    invoke.mockResolvedValue([]);
    await listReports("p1");
    expect(invoke).toHaveBeenCalledWith("reports_list", { projectId: "p1" });
  });

  it("readReport 傳 projectId 與 filename", async () => {
    invoke.mockResolvedValue("<html></html>");
    await readReport("p1", "a.html");
    expect(invoke).toHaveBeenCalledWith("reports_read", { projectId: "p1", filename: "a.html" });
  });
});
