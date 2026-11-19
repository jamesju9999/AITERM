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
      { username: "", password: "" },
    );
  });

  it("saves the upload folders one per line, ignoring blank lines", async () => {
    const onSave = mount();
    fill();
    fireEvent.change(screen.getByPlaceholderText(t.sys_upload_dirs_ph), { target: { value: "/a/b\n\n  /c  \n" } });
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ upload_dirs: ["/a/b", "/c"] }), expect.anything());
  });

  it("passes the typed username and password along separately from the connection, and shows that a password is already saved", async () => {
    const onSave = vi.fn(() => Promise.resolve());
    render(
      <LocaleProvider>
        <ConnectionForm initialLogin={{ username: "u1", has_password: true }} onSave={onSave} onCancel={() => {}} />
      </LocaleProvider>,
    );
    fill();
    expect((screen.getByLabelText(t.sys_login_user) as HTMLInputElement).value).toBe("u1");
    const pass = screen.getByLabelText(t.sys_login_pass) as HTMLInputElement;
    expect(pass.placeholder).toBe(t.sys_login_pass_saved);
    expect(pass.type).toBe("password");
    fireEvent.change(pass, { target: { value: "s3cret" } });
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave).toHaveBeenCalledWith(expect.not.objectContaining({ password: expect.anything() }), { username: "u1", password: "s3cret" });
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
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ mode: "autonomous" }), expect.anything()));
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

  it("saves the chosen model for the system, and keeps a custom model name that is already stored", async () => {
    const onSave = mount();
    fill();
    fireEvent.change(screen.getByLabelText(t.sys_model), { target: { value: "haiku" } });
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ model: "haiku" }), expect.anything()));
  });

  it("a system saved with a full model name shows it and does not lose it", async () => {
    const onSave = vi.fn(() => Promise.resolve());
    render(
      <LocaleProvider>
        <ConnectionForm
          initial={{ id: "s", name: "S", start_url: "https://a.test/", allowed_hosts: ["a.test"], mode: "read_only", rules: [], enabled: true, model: "claude-sonnet-5-5" }}
          onSave={onSave}
          onCancel={() => {}}
        />
      </LocaleProvider>,
    );
    expect((screen.getByLabelText(t.sys_model) as HTMLSelectElement).value).toBe("claude-sonnet-5-5");
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ model: "claude-sonnet-5-5" }), expect.anything()));
  });
  it("saves a web system with kind web and no bound app by default", async () => {
    const onSave = mount();
    fill();
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ kind: "web", app: "" }), expect.anything());
  });

  it("switches to a desktop app system: asks for the app, hides URL/domain/upload fields and saves them empty", async () => {
    const onSave = mount();
    fireEvent.click(screen.getByRole("radio", { name: t.sys_kind_desktop }));
    expect(screen.queryByPlaceholderText(t.sys_url_ph)).toBeNull();
    expect(screen.queryByPlaceholderText(t.sys_upload_dirs_ph)).toBeNull();
    fireEvent.change(screen.getByPlaceholderText(t.sys_name_ph), { target: { value: "進銷存" } });
    const save = screen.getByText(t.sys_save) as HTMLButtonElement;
    expect(save.disabled).toBe(true); // 還沒填 App
    fireEvent.change(screen.getByPlaceholderText(t.sys_app_ph), { target: { value: " ERP Client " } });
    expect(save.disabled).toBe(false);
    fireEvent.click(save);
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ kind: "desktop", app: "ERP Client", launch_app: true, start_url: "", allowed_hosts: [], upload_dirs: [], mode: "read_only" }),
      expect.anything(),
    );
  });

  it("lets a desktop system opt out of launching the app and offers desktop rule actions", async () => {
    const onSave = mount();
    fireEvent.click(screen.getByRole("radio", { name: t.sys_kind_desktop }));
    fireEvent.change(screen.getByPlaceholderText(t.sys_name_ph), { target: { value: "X" } });
    fireEvent.change(screen.getByPlaceholderText(t.sys_app_ph), { target: { value: "Notes" } });
    fireEvent.click(screen.getByLabelText(t.sys_launch_app));
    pickMode(t.sys_mode_title_rules);
    fireEvent.click(screen.getByText(t.sys_rule_add));
    const method = screen.getByLabelText(t.sys_rule_method) as HTMLSelectElement;
    expect(Array.from(method.options).map((o) => o.value)).toEqual(["*", "click", "type", "key", "login"]);
    expect(method.value).toBe("click");
    fireEvent.change(screen.getByLabelText(t.sys_rule_pattern), { target: { value: "刪除*" } });
    fireEvent.click(screen.getByText(t.sys_save));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ launch_app: false, rules: [{ method: "click", url_pattern: "刪除*", action: "allow" }] }),
      expect.anything(),
    );
  });
});
