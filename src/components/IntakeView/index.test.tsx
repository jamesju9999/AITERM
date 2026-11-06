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
  intakeProjects: vi.fn(),
  intakeAdoptLegacy: vi.fn(() => Promise.resolve()),
  intakeAssign: vi.fn(() => Promise.resolve(true)),
  intakeCopyLegacyConfig: vi.fn(() => Promise.resolve()),
  intakeRefresh: vi.fn(),
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
  intakeAttachmentText: vi.fn(),
}));
import * as ipc from "../../ipc/intake";

const t = translations["zh-TW"];
const webPort = `${t.intake_set_web_port} ${t.intake_web_port_shared}`;

function item(over: Partial<IntakeItem>): IntakeItem {
  return {
    id: "1", source: "email", sender: "bob@corp.com", sender_level: "external", subject: "壞掉了", body: "請修",
    message_id: "m", status: "pending", tab_id: null, reply_draft: null, error: null, created_at: 0, updated_at: 0,
    kind: null, snapshot_head: null, verification: null, verified: null, sender_name: null, project_id: "p1", ...over,
  };
}
const wrap = () => render(<LocaleProvider><IntakeView /></LocaleProvider>);

const P1 = { id: "p1", name: "專案一", repo_dir: "/work/one", open_count: 0, total: 0, enabled: true };
const P2 = { id: "p2", name: "專案二", repo_dir: null, open_count: 2, total: 5, enabled: false };

function cfgView(over: Partial<ipc.IntakeConfig> = {}, extra: Partial<ipc.IntakeConfigView> = {}): ipc.IntakeConfigView {
  return {
    config: {
      enabled: false, account_id: "", mode: "alias_or_tag", alias: "", tag: "[AITerm]", auth_server: "",
      internal_without_auth: false, dispatch_internal: "auto", dispatch_external: "manual",
      reply_internal_auto: true, reply_external_auto: false, rate_limit_per_hour: 10,
      verify_command: "", verify_timeout_secs: 600, telegram_enabled: false, web_mode: "off", web_port: 8320, ...over,
    },
    saved: true,
    telegram_default: false,
    ...extra,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  try {
    localStorage.clear();
  } catch {
    /* ignore */
  }
  vi.mocked(ipc.intakeProjects).mockResolvedValue({ projects: [P1, P2], unassigned: 0, legacy_settings: false });
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

  it("each flow step carries a tooltip saying what it is and where this question stands", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ status: "running" })]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    const steps = await screen.findAllByRole("tooltip");
    expect(steps).toHaveLength(6);
    const answer = steps[3];
    expect(answer).toHaveTextContent(t.intake_flow_answer);
    expect(answer).toHaveTextContent(t.intake_flow_desc_answer);
    expect(answer).toHaveTextContent(t.intake_flow_state_current);
    expect(steps[0]).toHaveTextContent(t.intake_flow_state_done);
    expect(steps[5]).toHaveTextContent(t.intake_flow_state_todo);
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

  it("lists non-image attachments with name and size, loads image data only for images, and previews text files", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([item({})]);
    vi.mocked(ipc.intakeAttachments).mockResolvedValue([
      { id: "i1", item_id: "1", filename: "1-shot.png", path: "/x/1-shot.png", mime: "image/png", size: 10 },
      { id: "f1", item_id: "1", filename: "2-server.log", path: "/x/2-server.log", mime: "text/plain", size: 2048 },
      { id: "f2", item_id: "1", filename: "3-report.pdf", path: "/x/3-report.pdf", mime: "application/octet-stream", size: 3 * 1024 * 1024 },
    ]);
    vi.mocked(ipc.intakeAttachmentData).mockResolvedValue("data:image/png;base64,AAAA");
    vi.mocked(ipc.intakeAttachmentText).mockResolvedValue("ERROR boom\n密碼錯誤");
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    expect(await screen.findByText("2-server.log")).toBeInTheDocument();
    expect(screen.getByText("3-report.pdf")).toBeInTheDocument();
    expect(screen.getByText("2.0 KB")).toBeInTheDocument();
    expect(screen.getByText("3.0 MB")).toBeInTheDocument();
    expect(screen.getByText(t.intake_attachments.replace("{n}", "3"))).toBeInTheDocument();
    await screen.findByAltText("1-shot.png");
    expect(ipc.intakeAttachmentData).toHaveBeenCalledTimes(1);
    expect(ipc.intakeAttachmentData).toHaveBeenCalledWith("i1");
    // 只有純文字有預覽鈕；pdf 沒有。
    const previews = screen.getAllByText(t.intake_file_preview);
    expect(previews).toHaveLength(1);
    fireEvent.click(previews[0]);
    expect(await screen.findByText(/ERROR boom/)).toBeInTheDocument();
    expect(ipc.intakeAttachmentText).toHaveBeenCalledWith("f1");
    fireEvent.click(screen.getByText(t.intake_file_hide));
    await waitFor(() => expect(screen.queryByText(/ERROR boom/)).toBeNull());
  });

  it("copy path puts the file's location on the clipboard", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    vi.mocked(ipc.intakeList).mockResolvedValue([item({})]);
    vi.mocked(ipc.intakeAttachments).mockResolvedValue([
      { id: "f2", item_id: "1", filename: "3-report.pdf", path: "/x/3-report.pdf", mime: "application/octet-stream", size: 3 },
    ]);
    wrap();
    fireEvent.click(await screen.findByText("壞掉了"));
    fireEvent.click(await screen.findByText(t.intake_file_copy_path));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("/x/3-report.pdf"));
    expect(await screen.findByText(t.intake_file_copied)).toBeInTheDocument();
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
    vi.mocked(ipc.intakeGetConfig).mockResolvedValue(cfgView({ enabled: false }));
    wrap();
    fireEvent.click(await screen.findByText(t.intake_settings));
    const addr = await screen.findByLabelText(t.intake_sender_address);
    const add = screen.getByText(t.intake_add) as HTMLButtonElement;
    expect(add.disabled).toBe(true);
    fireEvent.change(addr, { target: { value: "bob@corp.com" } });
    expect(add.disabled).toBe(false);
    fireEvent.click(add);
    await waitFor(() =>
      expect(ipc.intakeUpsertSender).toHaveBeenCalledWith("p1", { address: "bob@corp.com", level: "internal", skip_auth: false }),
    );
    fireEvent.click(screen.getByLabelText(t.intake_set_enabled));
    fireEvent.change(screen.getByLabelText(t.intake_set_verify_command), { target: { value: "npm test" } });
    fireEvent.click(screen.getByText(t.intake_set_save));
    await waitFor(() =>
      expect(ipc.intakeSetConfig).toHaveBeenCalledWith("p1", expect.objectContaining({ enabled: true, verify_command: "npm test" }), false),
    );
    expect(screen.getByText(/not_allowed 3/)).toBeInTheDocument();
  });

  it("web form: LAN mode warns about exposure and the port only shows when enabled", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    vi.mocked(ipc.intakeGetConfig).mockResolvedValue(cfgView({ enabled: true }));
    wrap();
    fireEvent.click(await screen.findByText(t.intake_settings));
    const select = (await screen.findByLabelText(t.intake_set_web)) as HTMLSelectElement;
    expect(screen.queryByLabelText(webPort)).toBeNull();
    expect(screen.queryByText(t.intake_web_lan_warning)).toBeNull();
    fireEvent.change(select, { target: { value: "local" } });
    expect(screen.getByLabelText(webPort)).toBeInTheDocument();
    expect(screen.queryByText(t.intake_web_lan_warning)).toBeNull();
    fireEvent.change(select, { target: { value: "lan" } });
    expect(screen.getByText(t.intake_web_lan_warning)).toBeInTheDocument();
  });

  it("loads the first project's questions, and switching project reloads that project's list", async () => {
    vi.mocked(ipc.intakeList).mockImplementation((pid) => Promise.resolve(pid === "p2" ? [item({ id: "9", subject: "二號專案的題目", project_id: "p2" })] : [item({})]));
    wrap();
    await screen.findByText("壞掉了");
    expect(ipc.intakeList).toHaveBeenCalledWith("p1", undefined);
    fireEvent.change(screen.getByLabelText(t.intake_project_label), { target: { value: "p2" } });
    expect(await screen.findByText("二號專案的題目")).toBeInTheDocument();
    expect(screen.queryByText("壞掉了")).toBeNull();
    expect(localStorage.getItem("aiterm.intake.project")).toBe("p2");
  });

  it("the project picker shows how many questions need attention", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    wrap();
    expect(await screen.findByRole("option", { name: "專案二 (2)" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "專案一" })).toBeInTheDocument();
  });

  it("remembers the chosen project across visits and falls back when it no longer exists", async () => {
    localStorage.setItem("aiterm.intake.project", "gone");
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    wrap();
    await waitFor(() => expect(ipc.intakeList).toHaveBeenCalledWith("p1", undefined));
    expect((screen.getByLabelText(t.intake_project_label) as HTMLSelectElement).value).toBe("p1");
  });

  it("with no project at all it says to create one instead of showing an empty queue", async () => {
    vi.mocked(ipc.intakeProjects).mockResolvedValue({ projects: [], unassigned: 0, legacy_settings: false });
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    wrap();
    expect(await screen.findByText(t.intake_no_projects)).toBeInTheDocument();
    expect(screen.queryByText(t.intake_settings)).toBeNull();
  });

  it("unassigned questions get their own view, can be moved one by one or all at once, and have no settings", async () => {
    vi.mocked(ipc.intakeProjects).mockResolvedValue({ projects: [P1, P2], unassigned: 3, legacy_settings: true });
    vi.mocked(ipc.intakeList).mockResolvedValue([item({ project_id: "" })]);
    wrap();
    fireEvent.change(await screen.findByLabelText(t.intake_project_label), { target: { value: "" } });
    expect(screen.queryByText(t.intake_settings)).toBeNull();
    // 全部歸給某專案
    const adoptSelects = await screen.findAllByLabelText(t.intake_assign_pick);
    fireEvent.change(adoptSelects[0], { target: { value: "p2" } });
    fireEvent.click(screen.getByText(t.intake_adopt_all));
    await waitFor(() => expect(ipc.intakeAdoptLegacy).toHaveBeenCalledWith("p2"));
    // 單題指派
    fireEvent.click(await screen.findByText("壞掉了"));
    const perItem = await screen.findAllByLabelText(t.intake_assign_pick);
    fireEvent.change(perItem[perItem.length - 1], { target: { value: "p1" } });
    fireEvent.click(screen.getByText(t.intake_assign));
    await waitFor(() => expect(ipc.intakeAssign).toHaveBeenCalledWith("1", "p1"));
  });

  it("settings belong to the chosen project and say which one is being edited", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    vi.mocked(ipc.intakeGetConfig).mockResolvedValue(cfgView());
    wrap();
    fireEvent.change(await screen.findByLabelText(t.intake_project_label), { target: { value: "p2" } });
    fireEvent.click(await screen.findByText(t.intake_settings));
    expect(await screen.findByText(t.intake_editing_project.replace("{name}", "專案二"))).toBeInTheDocument();
    expect(ipc.intakeGetConfig).toHaveBeenCalledWith("p2");
    expect(ipc.intakeSenders).toHaveBeenCalledWith("p2");
  });

  it("a conflict from the backend (same tag as another project) is shown instead of silently saved", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    vi.mocked(ipc.intakeGetConfig).mockResolvedValue(cfgView());
    vi.mocked(ipc.intakeSetConfig).mockRejectedValue(new Error("與專案「專案二」衝突：主旨標籤跟另一個專案相同"));
    wrap();
    fireEvent.click(await screen.findByText(t.intake_settings));
    fireEvent.click(await screen.findByText(t.intake_set_save));
    expect(await screen.findByRole("alert")).toHaveTextContent("與專案「專案二」衝突");
    expect(screen.queryByText(new RegExp(t.intake_set_saved))).toBeNull();
  });

  it("does not offer copying when there are no old settings left", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    vi.mocked(ipc.intakeGetConfig).mockResolvedValue(cfgView({}, { saved: false }));
    wrap();
    fireEvent.click(await screen.findByText(t.intake_settings));
    await screen.findByText(t.intake_editing_project.replace("{name}", "專案一"));
    expect(screen.queryByText(t.intake_copy_legacy)).toBeNull();
  });

  it("refresh checks the mailbox for this project, shows how many arrived, and blocks double clicks while it runs", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    let finish!: (v: ipc.IntakeRefreshResult) => void;
    vi.mocked(ipc.intakeRefresh).mockReturnValue(new Promise((r) => (finish = r)));
    wrap();
    const btn = (await screen.findByLabelText(t.intake_refresh)) as HTMLButtonElement;
    fireEvent.click(btn);
    await waitFor(() => expect(btn.disabled).toBe(true));
    fireEvent.click(btn);
    expect(ipc.intakeRefresh).toHaveBeenCalledTimes(1);
    expect(ipc.intakeRefresh).toHaveBeenCalledWith("p1");
    finish({ polled: true, ok: true, new_items: 2, error: null });
    expect(await screen.findByText(t.intake_refresh_new.replace("{n}", "2"))).toBeInTheDocument();
    expect(btn.disabled).toBe(false);
  });

  it("refresh says so when nothing new arrived, when no mailbox is configured, and shows a failure", async () => {
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    wrap();
    const btn = await screen.findByLabelText(t.intake_refresh);
    vi.mocked(ipc.intakeRefresh).mockResolvedValueOnce({ polled: true, ok: true, new_items: 0, error: null });
    fireEvent.click(btn);
    expect(await screen.findByText(t.intake_refresh_none)).toBeInTheDocument();
    vi.mocked(ipc.intakeRefresh).mockResolvedValueOnce({ polled: false, ok: true, new_items: 0, error: null });
    fireEvent.click(btn);
    expect(await screen.findByText(t.intake_refresh_reloaded)).toBeInTheDocument();
    vi.mocked(ipc.intakeRefresh).mockResolvedValueOnce({ polled: true, ok: false, new_items: 0, error: "信箱連線或收信失敗，請檢查信箱帳號設定" });
    fireEvent.click(btn);
    expect(await screen.findByRole("alert")).toHaveTextContent("信箱連線或收信失敗");
    expect(screen.queryByText(t.intake_refresh_none)).toBeNull();
  });

  it("offers to copy the pre-upgrade settings when this project has none and old settings remain, even with no unassigned questions", async () => {
    // 題目都已歸屬（unassigned 為 0），但舊設定與白名單還在：照樣要有複製入口。
    vi.mocked(ipc.intakeProjects).mockResolvedValue({ projects: [P1, P2], unassigned: 0, legacy_settings: true });
    vi.mocked(ipc.intakeList).mockResolvedValue([]);
    vi.mocked(ipc.intakeGetConfig).mockResolvedValue(cfgView({}, { saved: false }));
    wrap();
    fireEvent.click(await screen.findByText(t.intake_settings));
    fireEvent.click(await screen.findByText(t.intake_copy_legacy));
    await waitFor(() => expect(ipc.intakeCopyLegacyConfig).toHaveBeenCalledWith("p1"));
  });

  it("accepts telegram chat ids in the allowlist but rejects malformed addresses", () => {
    for (const ok of ["a@b.co", "tg:123", "TG:-100123"]) expect(validAddress(ok), ok).toBe(true);
    for (const bad of ["", "tg:", "tg:abc", "a@b", "a b@c.de", "@c.de"]) expect(validAddress(bad), bad).toBe(false);
  });
});
