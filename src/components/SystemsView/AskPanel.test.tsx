import { fireEvent, render, screen } from "@testing-library/react";
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

  it("saves a goal per system, refills it on click and removes it", async () => {
    localStorage.clear();
    mount();
    const box = screen.getByPlaceholderText(t.sys_ask_goal_ph) as HTMLTextAreaElement;
    fireEvent.change(box, { target: { value: "查剩餘特休" } });
    fireEvent.click(screen.getByText(t.sys_goal_save));
    expect((screen.getByText(t.sys_goal_save) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(box, { target: { value: "" } });
    fireEvent.click(screen.getByText("查剩餘特休"));
    expect(box.value).toBe("查剩餘特休");
    fireEvent.click(screen.getByLabelText(t.sys_goal_remove));
    expect(screen.queryByTestId("saved-goals")).toBeNull();
    expect(localStorage.getItem(`aiterm.systems.goals.${system.id}`)).toBe("[]");
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
