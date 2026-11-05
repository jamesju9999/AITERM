import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { IntakeView } from "./index";
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
  intakeUpdateDraft: vi.fn(() => Promise.resolve(true)),
  intakeSendReply: vi.fn(() => Promise.resolve()),
  intakeGetConfig: vi.fn(),
  intakeSetConfig: vi.fn(() => Promise.resolve()),
  intakeSenders: vi.fn(() => Promise.resolve([])),
  intakeUpsertSender: vi.fn(() => Promise.resolve()),
  intakeDeleteSender: vi.fn(() => Promise.resolve(true)),
  intakeDropped: vi.fn(() => Promise.resolve([["not_allowed", 3]])),
}));
import * as ipc from "../../ipc/intake";

const t = translations["zh-TW"];

function item(over: Partial<IntakeItem>): IntakeItem {
  return {
    id: "1", source: "email", sender: "bob@corp.com", sender_level: "external", subject: "壞掉了", body: "請修",
    message_id: "m", status: "pending", tab_id: null, reply_draft: null, error: null, created_at: 0, updated_at: 0, ...over,
  };
}
const wrap = () => render(<LocaleProvider><IntakeView /></LocaleProvider>);

beforeEach(() => {
  vi.clearAllMocks();
});

describe("IntakeView", () => {
  it("lists items with their status and lets a pending one be dispatched", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({})]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    fireEvent.click(await screen.findByText(t.intake_action_approve));
    await waitFor(() => expect(ipc.intakeApprove).toHaveBeenCalledWith("1"));
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

  it("a sent (done) reply is read-only", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ status: "done", reply_draft: "已寄" })]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    expect(((await screen.findByDisplayValue("已寄")) as HTMLTextAreaElement).readOnly).toBe(true);
    expect(screen.queryByText(t.intake_action_send)).toBeNull();
  });

  it("surfaces a backend failure instead of swallowing it", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ status: "draft", reply_draft: "x" })]);
    vi.mocked(ipc.intakeSendReply).mockRejectedValue(new Error("SMTP 認證失敗"));
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    fireEvent.click(await screen.findByText(t.intake_action_send));
    expect(await screen.findByRole("alert")).toHaveTextContent("SMTP 認證失敗");
  });

  it("settings: adds an allowlist sender only with a valid address and saves config", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    vi.mocked(ipc.intakeGetConfig).mockResolvedValue({
      enabled: false, account_id: "", mode: "alias_or_tag", alias: "", tag: "[AITerm]", auth_server: "",
      internal_without_auth: false, dispatch_internal: "auto", dispatch_external: "manual",
      reply_internal_auto: true, reply_external_auto: false, rate_limit_per_hour: 10,
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
    fireEvent.click(screen.getByText(t.intake_set_save));
    await waitFor(() => expect(ipc.intakeSetConfig).toHaveBeenCalledWith(expect.objectContaining({ enabled: true })));
    expect(screen.getByText(/not_allowed 3/)).toBeInTheDocument();
  });
});
