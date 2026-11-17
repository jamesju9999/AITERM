import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import * as ipc from "../../ipc/systems";
import type { SystemConnection } from "../../ipc/systems";
import { translations } from "../../lib/i18n";
import { SystemsSkillDialog } from "./SystemsSkillDialog";

vi.mock("../../ipc/systems", () => ({ systemsList: vi.fn() }));

const t = translations["zh-TW"];
const sys = (id: string, over: Partial<SystemConnection> = {}): SystemConnection => ({
  id, name: `系統${id}`, start_url: "https://x/", allowed_hosts: ["x"], mode: "autonomous", rules: [], enabled: true, ...over,
});

function mount(list: SystemConnection[], current: string[] = [], onSave = vi.fn(() => Promise.resolve())) {
  vi.mocked(ipc.systemsList).mockResolvedValue(list);
  const onClose = vi.fn();
  render(
    <LocaleProvider>
      <SystemsSkillDialog agentName="驗收者" current={current} onSave={onSave} onClose={onClose} />
    </LocaleProvider>,
  );
  return { onSave, onClose };
}

describe("SystemsSkillDialog", () => {
  it("explains there is nothing to pick when no system connection exists", async () => {
    mount([]);
    expect(await screen.findByText(t.ac_sys_none_defined)).toBeInTheDocument();
    expect((screen.getByText(t.ac_sys_save) as HTMLButtonElement).disabled).toBe(true);
  });

  it("lists systems with their permission, saves only the ticked ones and closes", async () => {
    const { onSave, onClose } = mount([sys("a"), sys("b", { mode: "read_only" })]);
    await screen.findByText(/系統a/);
    expect(screen.getByText(new RegExp(t.sys_mode_title_read_only))).toBeInTheDocument();
    fireEvent.click(screen.getAllByRole("checkbox")[1]);
    fireEvent.click(screen.getByText(t.ac_sys_save));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(["b"]));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("warns about unattended runs only when a read-only or approve-each system is ticked", async () => {
    mount([sys("a"), sys("b", { mode: "approve_each" })]);
    await screen.findByText(/系統a/);
    const [a, b] = screen.getAllByRole("checkbox");
    fireEvent.click(a);
    expect(screen.queryByTestId("systems-skill-warning")).toBeNull();
    fireEvent.click(b);
    expect(screen.getByTestId("systems-skill-warning")).toBeInTheDocument();
  });

  it("a disabled system cannot be newly ticked, and a removed one is reported; remove clears everything", async () => {
    const { onSave } = mount([sys("a", { enabled: false })], ["a", "ghost"]);
    await screen.findByText(/系統a/);
    expect(screen.getByText(t.ac_sys_missing("ghost"))).toBeInTheDocument();
    fireEvent.click(screen.getByText(t.ac_sys_remove));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith([]));
  });
});
