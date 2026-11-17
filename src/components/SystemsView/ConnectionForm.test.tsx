import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations } from "../../lib/i18n";
import { ConnectionForm } from "./ConnectionForm";
import { hostOf, slug } from "./utils";

const confirmMock = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: (...a: unknown[]) => confirmMock(...a) }));
vi.mock("../../lib/webviewFocus", () => ({ restoreWebviewFocus: () => Promise.resolve() }));

const t = translations["zh-TW"];

function mount(onSave = vi.fn(() => Promise.resolve())) {
  render(
    <LocaleProvider>
      <ConnectionForm onSave={onSave} onCancel={() => {}} />
    </LocaleProvider>,
  );
  return onSave;
}

const fill = () => {
  fireEvent.change(screen.getByPlaceholderText(t.sys_name_ph), { target: { value: "Orders" } });
  fireEvent.change(screen.getByPlaceholderText(t.sys_url_ph), { target: { value: "https://app.example.com/login" } });
};
const pickMode = (title: string) => fireEvent.click(screen.getByRole("radio", { name: new RegExp(title) }));

beforeEach(() => confirmMock.mockReset());

describe("ConnectionForm", () => {
  it("defaults the allowed domain to the start URL's domain and starts read-only", async () => {
    const onSave = mount();
    fill();
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ name: "Orders", allowed_hosts: ["app.example.com"], mode: "read_only", enabled: true, rules: [] }),
    );
  });

  it("will not save autonomous mode unless the user confirms", async () => {
    const onSave = mount();
    fill();
    pickMode(t.sys_mode_title_autonomous);
    confirmMock.mockResolvedValueOnce(false);
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(confirmMock).toHaveBeenCalledTimes(1));
    expect(onSave).not.toHaveBeenCalled();

    confirmMock.mockResolvedValueOnce(true);
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ mode: "autonomous" })));
  });

  it("only keeps rules in rules mode and drops blank patterns", async () => {
    const onSave = mount();
    fill();
    pickMode(t.sys_mode_title_rules);
    fireEvent.click(screen.getByText(t.sys_rule_add));
    fireEvent.click(screen.getByText(t.sys_rule_add));
    const patterns = screen.getAllByLabelText(t.sys_rule_pattern);
    fireEvent.change(patterns[0], { target: { value: "https://app.example.com/drafts/*" } });
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    const saved = (onSave.mock.calls as unknown as [{ rules: unknown[] }][])[0][0];
    expect(saved.rules).toEqual([{ method: "POST", url_pattern: "https://app.example.com/drafts/*", action: "allow" }]);
  });

  it("shows the backend's validation error", async () => {
    mount(vi.fn(() => Promise.reject("允許的網域必須包含起始網址的網域")));
    fill();
    fireEvent.click(screen.getByText(t.sys_save));
    expect(await screen.findByRole("alert")).toBeTruthy();
  });

  it("helpers", () => {
    expect(hostOf("https://App.Example.com:8080/x")).toBe("app.example.com");
    expect(hostOf("not a url")).toBe("");
    expect(slug("My Orders!")).toBe("my-orders");
  });

  it("explains each permission mode in plain words and starts on the safest one", () => {
    mount();
    expect(screen.getByRole("radio", { name: new RegExp(t.sys_mode_title_read_only) })).toBeChecked();
    expect(screen.getByText(t.sys_mode_desc_approve_each)).toBeInTheDocument();
    expect(screen.getByText(t.sys_risk_high)).toBeInTheDocument();
  });

  it("only enables save for a proper http(s) URL and shows which domain will be allowed", () => {
    mount();
    fireEvent.change(screen.getByPlaceholderText(t.sys_name_ph), { target: { value: "X" } });
    const save = screen.getByText(t.sys_save) as HTMLButtonElement;
    fireEvent.change(screen.getByPlaceholderText(t.sys_url_ph), { target: { value: "app.example.com" } });
    expect(save.disabled).toBe(true);
    expect(screen.getByText(t.sys_url_invalid)).toBeInTheDocument();
    fireEvent.change(screen.getByPlaceholderText(t.sys_url_ph), { target: { value: "https://app.example.com/x" } });
    expect(save.disabled).toBe(false);
    expect(screen.getByText(t.sys_hosts_default("app.example.com"))).toBeInTheDocument();
  });

  it("a rule can be removed", () => {
    mount();
    pickMode(t.sys_mode_title_rules);
    fireEvent.click(screen.getByText(t.sys_rule_add));
    expect(screen.getAllByTestId("rule-row").length).toBe(1);
    fireEvent.click(screen.getByLabelText(t.sys_rule_remove));
    expect(screen.queryByTestId("rule-row")).toBeNull();
  });
});
