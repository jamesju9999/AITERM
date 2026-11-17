import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { JobRun, SystemConnection, SystemJob } from "../../ipc/systems";
import { translations } from "../../lib/i18n";
import { JobList } from "./JobList";

const t = translations["zh-TW"];
const system = (mode: SystemConnection["mode"] = "autonomous"): SystemConnection => ({
  id: "s1", name: "訂單系統", start_url: "https://x/", allowed_hosts: ["x"], mode, rules: [], enabled: true,
});
const job = (over: Partial<SystemJob> = {}): SystemJob => ({
  id: "j1", system_id: "s1", name: "每日匯出", goal: "匯出昨天的訂單", kind: "daily",
  spec_json: JSON.stringify({ times: ["09:00"], days: [], every_minutes: 0 }),
  enabled: true, created_ms: 1, last_fired_ms: 1, fail_streak: 0, disabled_reason: null, ...over,
});
const run = (over: Partial<JobRun> = {}): JobRun => ({ id: 1, job_id: "j1", started_ms: 5, finished_ms: 6, status: "ok", summary: "完成", ...over });

function mount(over: Partial<React.ComponentProps<typeof JobList>> = {}) {
  const props = {
    system: system(), jobs: [job()], runs: { j1: [run()] },
    onSave: vi.fn(() => Promise.resolve()), onDelete: vi.fn(), onToggle: vi.fn(), onRunNow: vi.fn(), onShowRuns: vi.fn(),
    ...over,
  };
  render(<LocaleProvider><JobList {...props} /></LocaleProvider>);
  return props;
}

describe("JobList", () => {
  it("shows the schedule sentence and the last run's status", () => {
    mount();
    expect(screen.getByText("每天 09:00")).toBeTruthy();
    expect(screen.getByText(t.sys_job_status_ok)).toBeTruthy();
  });

  it("warns that read-only and approve-each systems cannot finish writes unattended, but not the others", () => {
    mount({ system: system("read_only") });
    expect(screen.getByTestId("job-warning").textContent).toContain("只讀");
  });
  it("warns for approve-each", () => {
    mount({ system: system("approve_each") });
    expect(screen.getByTestId("job-warning").textContent).toContain("逐次核准");
  });
  it("does not warn for autonomous", () => {
    mount();
    expect(screen.queryByTestId("job-warning")).toBeNull();
  });

  it("shows why a job was disabled", () => {
    mount({ jobs: [job({ enabled: false, disabled_reason: "連續失敗" })] });
    expect(screen.getByText(t.sys_job_disabled_reason("連續失敗"))).toBeTruthy();
  });

  it("run now, toggle and delete report the job", () => {
    const p = mount();
    fireEvent.click(screen.getByText(t.sys_job_run_now));
    fireEvent.click(screen.getByText(t.sys_disable));
    fireEvent.click(screen.getByText(t.sys_delete));
    expect(p.onRunNow).toHaveBeenCalledWith(expect.objectContaining({ id: "j1" }));
    expect(p.onToggle).toHaveBeenCalledWith(expect.objectContaining({ id: "j1" }));
    expect(p.onDelete).toHaveBeenCalledWith(expect.objectContaining({ id: "j1" }));
  });

  it("the run history opens with its summary", () => {
    const p = mount();
    expect(screen.queryByTestId("job-runs")).toBeNull();
    fireEvent.click(screen.getByText(t.sys_job_runs));
    expect(screen.getByTestId("job-runs").textContent).toContain("完成");
    expect(p.onShowRuns).toHaveBeenCalled();
  });

  it("creates a job with the schedule built from the form", async () => {
    const p = mount({ jobs: [] });
    fireEvent.click(screen.getByText(t.sys_job_add));
    const inputs = screen.getAllByRole("textbox");
    fireEvent.change(inputs[0], { target: { value: "每日匯出" } });
    fireEvent.change(inputs[1], { target: { value: "匯出昨天的訂單" } });
    fireEvent.change(inputs[2], { target: { value: "09:00, 18:30" } });
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(p.onSave).toHaveBeenCalled());
    expect(p.onSave).toHaveBeenCalledWith(
      { system_id: "s1", name: "每日匯出", goal: "匯出昨天的訂單", kind: "daily", spec: { times: ["09:00", "18:30"], days: [], every_minutes: 0, window: null } },
      undefined,
    );
  });

  it("shows the backend's validation error and keeps the form open", async () => {
    mount({ jobs: [], onSave: vi.fn(() => Promise.reject("至少要設定一個時間點")) });
    fireEvent.click(screen.getByText(t.sys_job_add));
    const inputs = screen.getAllByRole("textbox");
    fireEvent.change(inputs[0], { target: { value: "x" } });
    fireEvent.change(inputs[1], { target: { value: "y" } });
    fireEvent.click(screen.getByText(t.sys_save));
    expect(await screen.findByRole("alert")).toBeTruthy();
  });
});
