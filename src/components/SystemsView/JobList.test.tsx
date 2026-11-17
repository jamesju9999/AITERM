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

  it("creates a job from a goal, with the name filled in from the goal when left blank", async () => {
    const p = mount({ jobs: [] });
    fireEvent.click(screen.getByText(t.sys_job_add));
    fireEvent.change(screen.getByPlaceholderText(t.sys_job_goal_ph), { target: { value: "匯出昨天的訂單" } });
    expect(screen.getByTestId("job-preview").textContent).toContain("每天 09:00");
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(p.onSave).toHaveBeenCalled());
    expect(p.onSave).toHaveBeenCalledWith(
      { system_id: "s1", name: "匯出昨天的訂單", goal: "匯出昨天的訂單", kind: "daily", spec: { times: ["09:00"], days: [], every_minutes: 0, window: null } },
      undefined,
    );
  });

  it("times are chips: add one, remove one, and the preview follows", () => {
    mount({ jobs: [] });
    fireEvent.click(screen.getByText(t.sys_job_add));
    fireEvent.change(screen.getByLabelText(t.sys_job_times, { selector: "input" }), { target: { value: "18:30" } });
    fireEvent.click(screen.getByText(t.sys_job_add_time));
    expect(screen.getAllByTestId("job-time").map((e) => e.textContent)).toEqual(expect.arrayContaining([expect.stringContaining("09:00"), expect.stringContaining("18:30")]));
    expect(screen.getByTestId("job-preview").textContent).toContain("09:00、18:30");
    fireEvent.click(screen.getByLabelText(t.sys_job_time_remove("09:00")));
    expect(screen.getByTestId("job-preview").textContent).not.toContain("09:00");
    fireEvent.click(screen.getByLabelText(t.sys_job_time_remove("18:30")));
    expect(screen.getByText(t.sys_job_no_times)).toBeInTheDocument();
    expect((screen.getByText(t.sys_save) as HTMLButtonElement).disabled).toBe(true);
  });

  it("weekly needs at least one day, interval needs at least 5 minutes", () => {
    mount({ jobs: [] });
    fireEvent.click(screen.getByText(t.sys_job_add));
    fireEvent.change(screen.getByPlaceholderText(t.sys_job_goal_ph), { target: { value: "x" } });
    fireEvent.click(screen.getByRole("radio", { name: t.sys_job_kind_weekly }));
    for (const d of [1, 2, 3, 4, 5]) fireEvent.click(screen.getByText(t.sys_job_weekday(d)));
    expect((screen.getByText(t.sys_save) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByText(t.sys_job_weekday(6)));
    expect((screen.getByText(t.sys_save) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByRole("radio", { name: t.sys_job_kind_interval }));
    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "3" } });
    expect((screen.getByText(t.sys_save) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getByRole("spinbutton"), { target: { value: "30" } });
    expect((screen.getByText(t.sys_save) as HTMLButtonElement).disabled).toBe(false);
  });

  it("shows the backend's validation error and keeps the form open", async () => {
    mount({ jobs: [], onSave: vi.fn(() => Promise.reject("後端說不行")) });
    fireEvent.click(screen.getByText(t.sys_job_add));
    fireEvent.change(screen.getByPlaceholderText(t.sys_job_goal_ph), { target: { value: "y" } });
    fireEvent.click(screen.getByText(t.sys_save));
    expect(await screen.findByText(t.sys_error("後端說不行"))).toBeTruthy();
  });
});
