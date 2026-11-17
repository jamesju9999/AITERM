import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import * as ipc from "../../ipc/systems";
import type { PendingApproval, SystemConnection, SystemStatus } from "../../ipc/systems";
import { translations } from "../../lib/i18n";
import { SystemsView } from "./index";

const t = translations["zh-TW"];
const handlers: Record<string, (p: unknown) => void> = {};

vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(() => Promise.resolve(true)) }));
vi.mock("../../lib/webviewFocus", () => ({ restoreWebviewFocus: () => Promise.resolve() }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = (payload) => cb({ payload });
    return Promise.resolve(() => {});
  }),
}));
vi.mock("../../ipc/systems", async (orig) => ({
  ...(await orig<typeof import("../../ipc/systems")>()),
  systemsList: vi.fn(),
  systemsStatus: vi.fn(),
  systemsPending: vi.fn(),
  systemsAudit: vi.fn(() => Promise.resolve([])),
  systemsMcpConfig: vi.fn(() => Promise.resolve("{}")),
  systemsDecide: vi.fn(() => Promise.resolve(true)),
  systemsSave: vi.fn(() => Promise.resolve()),
  systemsStop: vi.fn(() => Promise.resolve(0)),
  systemsTakeover: vi.fn(() => Promise.resolve()),
  systemsHandback: vi.fn(() => Promise.resolve()),
  systemsOpenLogin: vi.fn(() => Promise.resolve()),
  systemsRemove: vi.fn(() => Promise.resolve()),
  systemsRecipes: vi.fn(() => Promise.resolve([])),
  systemsRunGoal: vi.fn(),
  systemsJournalDiscard: vi.fn(() => Promise.resolve()),
  systemsRecipeSaveFromJournal: vi.fn(() => Promise.resolve("ok")),
  systemsRecipeRun: vi.fn(() => Promise.resolve("已完成")),
  systemsJobs: vi.fn(() => Promise.resolve([])),
  systemsJobRuns: vi.fn(() => Promise.resolve([])),
  systemsJobSave: vi.fn(() => Promise.resolve("j1")),
  systemsJobDelete: vi.fn(() => Promise.resolve()),
  systemsJobSetEnabled: vi.fn(() => Promise.resolve()),
  systemsJobRunNow: vi.fn(() => Promise.resolve()),
  systemsRecipeDelete: vi.fn(() => Promise.resolve(true)),
}));

const conn = (over: Partial<SystemConnection> = {}): SystemConnection => ({
  id: "s1",
  name: "訂單系統",
  start_url: "https://app.example.com/",
  allowed_hosts: ["example.com"],
  mode: "approve_each",
  rules: [],
  enabled: true,
  ...over,
});
const st = (over: Partial<SystemStatus> = {}): SystemStatus => ({ running: true, manual: false, pending: 0, needs_login: false, ...over });
const approval: PendingApproval = { id: "a1", system_id: "s1", intent: "寫：送出訂單", method: "POST", url: "https://app.example.com/orders", declared_read: false, ts_ms: 1 };

function setup(list = [conn()], status = st(), pending: PendingApproval[] = []) {
  vi.mocked(ipc.systemsList).mockResolvedValue(list);
  vi.mocked(ipc.systemsStatus).mockResolvedValue(status);
  vi.mocked(ipc.systemsPending).mockResolvedValue(pending);
  return render(
    <LocaleProvider>
      <SystemsView />
    </LocaleProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  for (const k of Object.keys(handlers)) delete handlers[k];
});

describe("SystemsView", () => {
  it("shows the empty state, and lists connections with their status", async () => {
    setup([]);
    expect(await screen.findByText(t.sys_empty)).toBeTruthy();
  });

  it("shows running, takeover and needs-login states", async () => {
    setup([conn()], st({ needs_login: true }));
    expect((await screen.findByTestId("system-status")).textContent).toBe(t.sys_status_needs_login);
  });

  it("an approval event shows a card, and approving reports the decision and removes it", async () => {
    setup();
    await screen.findByText("訂單系統");
    await waitFor(() => expect(handlers["systems://need-approval"]).toBeTruthy());
    act(() => handlers["systems://need-approval"]({ id: "a1", pending: { system_id: "s1", intent: "寫：送出訂單", method: "POST", url: "https://app.example.com/orders", declared_read: false, ts_ms: 1 } }));
    expect(await screen.findByTestId("approval-card")).toBeTruthy();
    fireEvent.click(screen.getByText(t.sys_approve));
    await waitFor(() => expect(ipc.systemsDecide).toHaveBeenCalledWith("a1", true));
    expect(screen.queryByTestId("approval-card")).toBeNull();
  });

  it("the same approval arriving twice shows one card", async () => {
    setup([conn()], st(), [approval]);
    await screen.findByTestId("approval-card");
    act(() => handlers["systems://need-approval"]({ id: "a1", pending: { ...approval } }));
    expect(screen.getAllByTestId("approval-card").length).toBe(1);
  });

  it("reject-and-disable rejects, then saves the system as disabled", async () => {
    setup([conn()], st(), [approval]);
    fireEvent.click(await screen.findByText(t.sys_reject_disable));
    await waitFor(() => expect(ipc.systemsDecide).toHaveBeenCalledWith("a1", false));
    await waitFor(() => expect(ipc.systemsSave).toHaveBeenCalledWith(expect.objectContaining({ id: "s1", enabled: false })));
  });

  it("a resolved event removes the card", async () => {
    setup([conn()], st(), [approval]);
    await screen.findByTestId("approval-card");
    act(() => handlers["systems://resolved"]({ id: "a1" }));
    expect(screen.queryByTestId("approval-card")).toBeNull();
  });

  it("stop, takeover and hand back call the matching commands", async () => {
    setup();
    fireEvent.click(await screen.findByText(t.sys_stop));
    await waitFor(() => expect(ipc.systemsStop).toHaveBeenCalledWith("s1"));
    fireEvent.click(screen.getByText(t.sys_takeover));
    await waitFor(() => expect(ipc.systemsTakeover).toHaveBeenCalledWith("s1"));
  });

  it("while the user has taken over, the button hands control back", async () => {
    setup([conn()], st({ manual: true }));
    fireEvent.click(await screen.findByText(t.sys_handback));
    await waitFor(() => expect(ipc.systemsHandback).toHaveBeenCalledWith("s1"));
  });

  it("disabling a system saves it with enabled=false", async () => {
    setup();
    fireEvent.click(await screen.findByText(t.sys_disable));
    await waitFor(() => expect(ipc.systemsSave).toHaveBeenCalledWith(expect.objectContaining({ id: "s1", enabled: false })));
  });

  it("the recipes button loads and shows that system's recipes, and deleting asks then calls the command", async () => {
    vi.mocked(ipc.systemsRecipes).mockResolvedValue([
      { id: 9, system_id: "s1", name: "送單", description: "", params: [], steps: [], ok_count: 0, fail_count: 0, last_run_ms: null },
    ]);
    setup();
    fireEvent.click(await screen.findByText(t.sys_recipes_title));
    expect(await screen.findByTestId("recipe-row")).toBeTruthy();
    expect(ipc.systemsRecipes).toHaveBeenCalledWith("s1");
    const row = await screen.findByTestId("recipe-row");
    fireEvent.click(row.querySelectorAll("button")[2]);
    await waitFor(() => expect(ipc.systemsRecipeDelete).toHaveBeenCalledWith(9));
  });

  it("the jobs button loads that system's jobs, and a job-disabled event shows a notice", async () => {
    vi.mocked(ipc.systemsJobs).mockResolvedValue([
      { id: "j1", system_id: "s1", name: "每日匯出", goal: "g", kind: "daily", spec_json: '{"times":["09:00"]}', enabled: true, created_ms: 1, last_fired_ms: 1, fail_streak: 0, disabled_reason: null },
    ]);
    setup();
    fireEvent.click(await screen.findByText(t.sys_jobs_title, { selector: "button" }));
    expect(await screen.findByTestId("job-row")).toBeTruthy();
    expect(ipc.systemsJobs).toHaveBeenCalledWith("s1");
    await waitFor(() => expect(handlers["systems://job-disabled"]).toBeTruthy());
    act(() => handlers["systems://job-disabled"]({ name: "每日匯出" }));
    expect((await screen.findByTestId("job-notice")).textContent).toBe(t.sys_job_disabled_toast("每日匯出"));
  });

  it("the Ask AI tab runs a goal on that system, and a recipe can be run from the recipes tab", async () => {
    vi.mocked(ipc.systemsRunGoal).mockResolvedValue({ status: "ok", summary: "做好了", steps: [], params: [], can_save: false });
    vi.mocked(ipc.systemsRecipes).mockResolvedValue([
      { id: 9, system_id: "s1", name: "送單", description: "", params: [], steps: [], ok_count: 0, fail_count: 0, last_run_ms: null },
    ]);
    setup();
    fireEvent.click(await screen.findByText(t.sys_ask_title, { selector: "button" }));
    fireEvent.change(await screen.findByPlaceholderText(t.sys_ask_goal_ph), { target: { value: "查今天的訂單" } });
    fireEvent.click(screen.getByText(t.sys_ask_send));
    await waitFor(() => expect(ipc.systemsRunGoal).toHaveBeenCalledWith("s1", "查今天的訂單"));
    expect(await screen.findByText("做好了")).toBeTruthy();

    fireEvent.click(screen.getByText(t.sys_recipes_title, { selector: "button" }));
    fireEvent.click(await screen.findByTestId("recipe-run"));
    fireEvent.click(screen.getByText(t.sys_recipe_run_go));
    await waitFor(() => expect(ipc.systemsRecipeRun).toHaveBeenCalledWith("s1", "送單", {}));
  });
});
