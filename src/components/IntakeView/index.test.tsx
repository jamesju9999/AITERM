import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { IntakeView } from "./index";
import { validAddress } from "../../lib/intakeAddress";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translations } from "../../lib/i18n";
import type { IntakeItem } from "../../ipc/intake";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock("../../ipc/mail", () => ({ mailListAccounts: vi.fn(() => Promise.resolve([])) }));
vi.mock("../../ipc/intake", async (orig) => ({
  ...(await orig<typeof import("../../ipc/intake")>()),
  intakeList: vi.fn(),
  intakeApprove: vi.fn(() => Promise.resolve(true)),
  intakeReject: vi.fn(() => Promise.resolve(true)),
  intakeDelete: vi.fn(() => Promise.resolve(true)),
  intakeUpdateDraft: vi.fn(() => Promise.resolve(true)),
  intakeSendReply: vi.fn(() => Promise.resolve()),
  intakeGetConfig: vi.fn(),
  intakeSetConfig: vi.fn(() => Promise.resolve()),
  intakeSenders: vi.fn(() => Promise.resolve([])),
  intakeUpsertSender: vi.fn(() => Promise.resolve()),
  intakeDeleteSender: vi.fn(() => Promise.resolve(true)),
  intakeDropped: vi.fn(() => Promise.resolve([["not_allowed", 3]])),
  intakeAttachments: vi.fn(() => Promise.resolve([])),
  intakeAttachmentData: vi.fn(),
}));
import * as ipc from "../../ipc/intake";

const t = translations["zh-TW"];

function item(over: Partial<IntakeItem>): IntakeItem {
  return {
    id: "1", source: "email", sender: "bob@corp.com", sender_level: "external", subject: "壞掉了", body: "請修",
    message_id: "m", status: "pending", tab_id: null, reply_draft: null, error: null, created_at: 0, updated_at: 0,
    kind: null, snapshot_head: null, verification: null, verified: null, sender_name: null, ...over,
  };
}
const wrap = () => render(<LocaleProvider><IntakeView /></LocaleProvider>);

beforeEach(() => {
  vi.clearAllMocks();
  // clearAllMocks 不會清掉 mockResolvedValue 設過的實作，上一個測試的附圖會漏進來。
  vi.mocked(ipc.intakeAttachments).mockResolvedValue([]);
});

describe("IntakeView", () => {
  it("lists items with their status and lets a pending one be dispatched", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({})]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    fireEvent.click(await screen.findByText(t.intake_action_approve));
    await waitFor(() => expect(ipc.intakeApprove).toHaveBeenCalledWith("1"));
  });

  it("names who asked, with their level, in both the list and the detail", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ sender: "bob@corp.com", sender_level: "external" })]);
    wrap();
    const row = await screen.findByText(`${t.intake_asker}：bob@corp.com · ${t.intake_level_external}`);
    fireEvent.click(row);
    expect(await screen.findByText("bob@corp.com", { selector: "strong" })).toBeTruthy();
    expect(screen.getAllByText(t.intake_level_external).length).toBeGreaterThan(0);
  });

  it("shows the display name first, with the address beside it", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ sender_name: "王小明" })]);
    wrap();
    const row = await screen.findByText(`${t.intake_asker}：王小明 <bob@corp.com> · ${t.intake_level_external}`);
    fireEvent.click(row);
    expect(await screen.findByText("王小明", { selector: "strong" })).toBeTruthy();
    expect(screen.getByText("bob@corp.com", { selector: ".intake__asker .intake__meta" })).toBeTruthy();
  });

  it("delete needs a second click, and is not offered while the item is running", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ status: "draft", reply_draft: "x" })]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    fireEvent.click(await screen.findByText(t.intake_action_delete));
    expect(ipc.intakeDelete).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByText(t.intake_action_delete_confirm));
    await waitFor(() => expect(ipc.intakeDelete).toHaveBeenCalledWith("1"));
  });

  it("the list row has its own delete button, also two-step, without selecting the row", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ status: "draft", reply_draft: "x" })]);
    wrap();
    const del = await screen.findByLabelText(t.intake_action_delete);
    fireEvent.click(del);
    expect(ipc.intakeDelete).not.toHaveBeenCalled();
    expect(screen.queryByText("請修")).toBeNull(); // 沒有因為點刪除而選取該列
    fireEvent.click(await screen.findByText(t.intake_action_delete_confirm));
    await waitFor(() => expect(ipc.intakeDelete).toHaveBeenCalledWith("1"));
  });

  it("each list row shows when the question arrived", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ created_at: 1762396459 })]);
    const { container } = wrap();
    await screen.findByText("壞掉了");
    const time = container.querySelector(".intake__row time") as HTMLTimeElement;
    expect(time.dateTime).toBe(new Date(1762396459 * 1000).toISOString());
    expect(time.textContent).toMatch(/\d/);
  });

  it("a running item cannot be deleted", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ status: "running" })]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    await screen.findByText("請修");
    expect(screen.queryByText(t.intake_action_delete)).toBeNull();
  });

  it("does not offer dispatch for an item that is already running", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ status: "running" })]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    await screen.findByText("請修");
    expect(screen.queryByText(t.intake_action_approve)).toBeNull();
  });

  it("sends the edited draft: saves it first, then sends", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ status: "draft", reply_draft: "舊答案" })]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    const box = (await screen.findByDisplayValue("舊答案")) as HTMLTextAreaElement;
    fireEvent.change(box, { target: { value: "新答案" } });
    fireEvent.click(screen.getByText(t.intake_action_send));
    await waitFor(() => expect(ipc.intakeSendReply).toHaveBeenCalledWith("1"));
    expect(ipc.intakeUpdateDraft).toHaveBeenCalledWith("1", "新答案");
    const order = [
      vi.mocked(ipc.intakeUpdateDraft).mock.invocationCallOrder[0],
      vi.mocked(ipc.intakeSendReply).mock.invocationCallOrder[0],
    ];
    expect(order[0]).toBeLessThan(order[1]);
  });

  it("a sent (done) reply is shown rendered and cannot be edited", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ status: "done", reply_draft: "**已寄**" })]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    expect(await screen.findByText("已寄", { selector: "strong" })).toBeTruthy();
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByText(t.intake_action_send)).toBeNull();
  });

  it("a draft can be previewed as rendered Markdown without losing the edit", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ status: "draft", reply_draft: "**重點**" })]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    expect(((await screen.findByDisplayValue("**重點**")) as HTMLTextAreaElement).readOnly).toBe(false);
    fireEvent.click(screen.getByText(t.intake_preview));
    expect(await screen.findByText("重點", { selector: "strong" })).toBeTruthy();
    fireEvent.click(screen.getByText(t.intake_edit));
    expect(await screen.findByDisplayValue("**重點**")).toBeTruthy();
  });

  it("surfaces a backend failure instead of swallowing it", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ status: "draft", reply_draft: "x" })]);
    vi.mocked(ipc.intakeSendReply).mockRejectedValue(new Error("SMTP 認證失敗"));
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    fireEvent.click(await screen.findByText(t.intake_action_send));
    expect(await screen.findByRole("alert")).toHaveTextContent("SMTP 認證失敗");
  });

  it("shows the images the sender attached, and says so when one cannot be loaded", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({})]);
    vi.mocked(ipc.intakeAttachments).mockResolvedValue([
      { id: "a1", item_id: "1", filename: "1-shot.png", path: "/x/1-shot.png", mime: "image/png", size: 10 },
      { id: "a2", item_id: "1", filename: "2-bad.png", path: "/x/2-bad.png", mime: "image/png", size: 10 },
    ]);
    vi.mocked(ipc.intakeAttachmentData).mockImplementation((id) =>
      id === "a1" ? Promise.resolve("data:image/png;base64,AAAA") : Promise.reject(new Error("gone")),
    );
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    const img = (await screen.findByAltText("1-shot.png")) as HTMLImageElement;
    expect(img.src).toBe("data:image/png;base64,AAAA");
    expect(await screen.findByText(t.intake_attachment_failed)).toBeInTheDocument();
    expect(screen.getByText(t.intake_attachments.replace("{n}", "2"))).toBeInTheDocument();
  });

  it("clicking a thumbnail opens it enlarged; Esc and the close button dismiss it", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({})]);
    vi.mocked(ipc.intakeAttachments).mockResolvedValue([
      { id: "a1", item_id: "1", filename: "1-shot.png", path: "/x/1-shot.png", mime: "image/png", size: 10 },
    ]);
    vi.mocked(ipc.intakeAttachmentData).mockResolvedValue("data:image/png;base64,AAAA");
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.click(await screen.findByTitle("1-shot.png"));
    const dialog = await screen.findByRole("dialog");
    expect(dialog.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,AAAA");
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    fireEvent.click(await screen.findByTitle("1-shot.png"));
    fireEvent.click(await screen.findByLabelText(t.intake_image_close));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("shows no image section for an item without attachments", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({})]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    await screen.findByText("請修");
    expect(screen.queryByText(/附圖（/)).toBeNull();
  });

  it("shows the verification report, and marks a failed one as failed", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([
      item({ status: "draft", reply_draft: "r", kind: "code_change", verified: false, verification: "測試未通過：npm test" }),
    ]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    expect(await screen.findByText("測試未通過：npm test")).toBeInTheDocument();
    expect(screen.getByText(t.intake_verify_failed)).toBeInTheDocument();
    expect(screen.queryByText(t.intake_verify_passed)).toBeNull();
    expect(screen.getByText(t.intake_kind_code_change)).toBeInTheDocument();
  });

  it("settings: adds an allowlist sender only with a valid address and saves config", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    vi.mocked(ipc.intakeGetConfig).mockResolvedValue({
      enabled: false, account_id: "", mode: "alias_or_tag", alias: "", tag: "[AITerm]", auth_server: "",
      internal_without_auth: false, dispatch_internal: "auto", dispatch_external: "manual",
      reply_internal_auto: true, reply_external_auto: false, rate_limit_per_hour: 10,
      verify_command: "", verify_timeout_secs: 600, telegram_enabled: false, web_mode: "off", web_port: 8320,
    });
    wrap();
    fireEvent.click(await screen.findByText(t.intake_settings));
    const addr = await screen.findByLabelText(t.intake_sender_address);
    const add = screen.getByText(t.intake_add) as HTMLButtonElement;
    expect(add.disabled).toBe(true);
    fireEvent.change(addr, { target: { value: "bob@corp.com" } });
    expect(add.disabled).toBe(false);
    fireEvent.click(add);
    await waitFor(() =>
      expect(ipc.intakeUpsertSender).toHaveBeenCalledWith({ address: "bob@corp.com", level: "internal", skip_auth: false }),
    );
    fireEvent.click(screen.getByLabelText(t.intake_set_enabled));
    fireEvent.change(screen.getByLabelText(t.intake_set_verify_command), { target: { value: "npm test" } });
    fireEvent.click(screen.getByText(t.intake_set_save));
    await waitFor(() =>
      expect(ipc.intakeSetConfig).toHaveBeenCalledWith(expect.objectContaining({ enabled: true, verify_command: "npm test" })),
    );
    expect(screen.getByText(/not_allowed 3/)).toBeInTheDocument();
  });

  it("web form: LAN mode warns about exposure and the port only shows when enabled", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    vi.mocked(ipc.intakeGetConfig).mockResolvedValue({
      enabled: true, account_id: "", mode: "alias_or_tag", alias: "", tag: "[AITerm]", auth_server: "",
      internal_without_auth: false, dispatch_internal: "auto", dispatch_external: "manual",
      reply_internal_auto: true, reply_external_auto: false, rate_limit_per_hour: 10,
      verify_command: "", verify_timeout_secs: 600, telegram_enabled: false, web_mode: "off", web_port: 8320,
    });
    wrap();
    fireEvent.click(await screen.findByText(t.intake_settings));
    const select = (await screen.findByLabelText(t.intake_set_web)) as HTMLSelectElement;
    expect(screen.queryByLabelText(t.intake_set_web_port)).toBeNull();
    expect(screen.queryByText(t.intake_web_lan_warning)).toBeNull();
    fireEvent.change(select, { target: { value: "local" } });
    expect(screen.getByLabelText(t.intake_set_web_port)).toBeInTheDocument();
    expect(screen.queryByText(t.intake_web_lan_warning)).toBeNull();
    fireEvent.change(select, { target: { value: "lan" } });
    expect(screen.getByText(t.intake_web_lan_warning)).toBeInTheDocument();
  });

  it("accepts telegram chat ids in the allowlist but rejects malformed addresses", () => {
    for (const ok of ["a@b.co", "tg:123", "TG:-100123"]) expect(validAddress(ok), ok).toBe(true);
    for (const bad of ["", "tg:", "tg:abc", "a@b", "a b@c.de", "@c.de"]) expect(validAddress(bad), bad).toBe(false);
  });
});
