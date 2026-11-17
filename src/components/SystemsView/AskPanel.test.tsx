import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { AdhocReport, SystemConnection } from "../../ipc/systems";
import { translations } from "../../lib/i18n";
import { AskPanel } from "./AskPanel";

vi.mock("../../ipc/ai", () => ({ invokeAiComplete: vi.fn() }));
vi.mock("../../ipc/provider", () => ({ listProviders: vi.fn().mockResolvedValue([]) }));

const t = translations["zh-TW"];
const system: SystemConnection = { id: "s1", name: "訂單系統", start_url: "https://x/", allowed_hosts: ["x"], mode: "autonomous", rules: [], enabled: true };
const report = (over: Partial<AdhocReport> = {}): AdhocReport => ({
  status: "ok",
  summary: "單號 A-102 已送出。",
  steps: [
    { tool: "sys_type", intent: "寫：填單號", detail: "單號 · A-102", write: true },
    { tool: "sys_click", intent: "寫：送出", detail: "送出", write: true },
  ],
  params: [{ name: "單號", example: "A-102" }],
  can_save: true,
  ...over,
});

function mount(over: Partial<React.ComponentProps<typeof AskPanel>> = {}) {
  const props = {
    system,
    onRun: vi.fn(() => Promise.resolve(report())),
    onSaveRecipe: vi.fn(() => Promise.resolve("ok")),
    onDiscard: vi.fn(() => Promise.resolve()),
    ...over,
  };
  render(
    <LocaleProvider>
      <AskPanel {...props} />
    </LocaleProvider>,
  );
  return props;
}

const ask = async (goal = "填單號 A-102 並送出") => {
  fireEvent.change(screen.getByPlaceholderText(t.sys_ask_goal_ph), { target: { value: goal } });
  fireEvent.click(screen.getByText(t.sys_ask_send));
  await screen.findByTestId("ask-result");
};

describe("AskPanel", () => {
  it("cannot start without a goal, then runs it and shows the report and the steps with read/write marks", async () => {
    const p = mount();
    expect((screen.getByText(t.sys_ask_send) as HTMLButtonElement).disabled).toBe(true);
    await ask();
    expect(p.onRun).toHaveBeenCalledWith("填單號 A-102 並送出", null);
    expect(screen.getByText("單號 A-102 已送出。")).toBeInTheDocument();
    const steps = screen.getByTestId("ask-steps");
    expect(steps.textContent).toContain(t.sys_tool_type);
    expect(steps.textContent).toContain(t.sys_tool_click);
    expect(steps.textContent).toContain("單號 · A-102");
    expect(steps.textContent).toContain(t.sys_step_write);
  });

  it("shows a running state while the AI works", async () => {
    let finish: (r: AdhocReport) => void = () => {};
    mount({ onRun: vi.fn(() => new Promise<AdhocReport>((res) => (finish = res))) });
    fireEvent.change(screen.getByPlaceholderText(t.sys_ask_goal_ph), { target: { value: "x" } });
    fireEvent.click(screen.getByText(t.sys_ask_send));
    expect(await screen.findByTestId("ask-running")).toBeInTheDocument();
    expect((screen.getByText(t.sys_ask_send) as HTMLButtonElement).disabled).toBe(true);
    finish(report());
    await screen.findByTestId("ask-result");
    expect(screen.queryByTestId("ask-running")).toBeNull();
  });

  it("offers to save only after a successful run with steps", async () => {
    mount({ onRun: vi.fn(() => Promise.resolve(report({ status: "failed", summary: "找不到欄位", can_save: false, params: [] }))) });
    await ask();
    expect(screen.getByText(t.sys_ask_failed)).toBeInTheDocument();
    expect(screen.queryByTestId("ask-save")).toBeNull();
  });

  it("saves the recorded steps with the chosen parameter names, skipping unchecked candidates", async () => {
    const p = mount({
      onRun: vi.fn(() =>
        Promise.resolve(report({ params: [{ name: "單號", example: "A-102" }, { name: "備註", example: "急件" }] })),
      ),
    });
    await ask();
    fireEvent.change(screen.getByPlaceholderText(t.sys_save_name_ph), { target: { value: "送單" } });
    const rows = screen.getAllByTestId("ask-param");
    expect(rows.length).toBe(2);
    fireEvent.click(rows[1].querySelector('input[type="checkbox"]')!); // 不要「備註」
    fireEvent.change(rows[0].querySelectorAll("input")[1], { target: { value: "order_no" } });
    fireEvent.click(screen.getByText(t.sys_save_btn));
    await waitFor(() => expect(p.onSaveRecipe).toHaveBeenCalledWith("送單", "", [{ name: "order_no", example: "A-102" }]));
    expect((await screen.findByTestId("ask-saved")).textContent).toBe(t.sys_save_done("送單"));
    expect(screen.queryByTestId("ask-result")).toBeNull();
  });

  it("needs a recipe name to save, and 'don't save' discards the recording", async () => {
    const p = mount();
    await ask();
    expect((screen.getByText(t.sys_save_btn) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByText(t.sys_save_skip));
    await waitFor(() => expect(p.onDiscard).toHaveBeenCalled());
    expect(screen.queryByTestId("ask-result")).toBeNull();
  });

  it("explains when nothing looked like a changing value", async () => {
    mount({ onRun: vi.fn(() => Promise.resolve(report({ params: [] }))) });
    await ask();
    expect(screen.getByText(t.sys_save_no_params)).toBeInTheDocument();
  });

  it("shows the error when the run itself fails (for example the system is busy)", async () => {
    mount({ onRun: vi.fn(() => Promise.reject("這個系統正在處理別的事")) });
    fireEvent.change(screen.getByPlaceholderText(t.sys_ask_goal_ph), { target: { value: "x" } });
    fireEvent.click(screen.getByText(t.sys_ask_send));
    expect(await screen.findByText(t.sys_error("這個系統正在處理別的事"))).toBeInTheDocument();
  });

  it("uses the system's model by default and lets this run pick another one", async () => {
    const p = mount({ system: { ...system, model: "haiku" } });
    const select = screen.getByLabelText(t.sys_model) as HTMLSelectElement;
    expect(select.value).toBe("");
    expect(select.options[0].textContent).toBe(t.sys_model_inherit_system("Haiku"));
    fireEvent.change(select, { target: { value: "opus" } });
    await ask("做一件複雜的事");
    expect(p.onRun).toHaveBeenCalledWith("做一件複雜的事", "opus");
  });
});
