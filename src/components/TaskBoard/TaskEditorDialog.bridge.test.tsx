import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const usedDirs = vi.fn();
const createTask = vi.fn();
const updateTask = vi.fn();
const bridgeStatus = vi.fn();
vi.mock("../../ipc/projects", () => ({ usedDirs: (...a: unknown[]) => usedDirs(...a) }));
vi.mock("../../ipc/tasks", () => ({
  createTask: (...a: unknown[]) => createTask(...a),
  updateTask: (...a: unknown[]) => updateTask(...a),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));
vi.mock("../../ipc/bridge", () => ({ bridgeStatus: (...a: unknown[]) => bridgeStatus(...a) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { saveBridgeProfiles } from "../Settings/bridgeProfiles";
import { TaskEditorDialog } from "./TaskEditorDialog";
import type { TaskWithAttachments } from "../../ipc/tasks";

const PROFILE = {
  id: "p1",
  name: "個人帳號",
  opus: { provider_id: "acct-a", model: "m-a" },
  sonnet: null,
  haiku: null,
};

const mount = (card: TaskWithAttachments | null = null) =>
  render(
    <LocaleProvider>
      <TaskEditorDialog projectId="p1" card={card} onClose={vi.fn()} onSaved={vi.fn()} />
    </LocaleProvider>,
  );

const BASE_CARD: TaskWithAttachments = {
  id: "t1",
  title: "既有卡片",
  body: "body",
  project_dir: "/repo",
  status: "planning",
  parallel_ok: true,
  interactive: false,
  sort_order: 1,
  outcome: null,
  tab_id: null,
  transcript_path: null,
  error_message: null,
  created_at: "2026-01-01",
  dispatched_at: null,
  finished_at: null,
  ai_summary: null,
  archived_at: null,
  session_id: null,
  session_path: null,
  use_bridge: false,
  bridge_tiers: null,
  label: null,
  attachments: [],
};

describe("TaskEditorDialog 派工方式", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    usedDirs.mockResolvedValue([]);
    bridgeStatus.mockResolvedValue({ running: true, port: 8317, token: "tok", error: null });
  });

  it("預設是「不走橋接」，選單列出已存的帳號組合", async () => {
    saveBridgeProfiles([PROFILE]);
    mount();
    const select = await screen.findByTestId("task-bridge-select");
    expect(select).toHaveValue("direct");
    expect(screen.getByRole("option", { name: "走橋接：個人帳號" })).toBeInTheDocument();
  });

  it("選帳號組合存檔時，帶入該組合解析後的 JSON 快照", async () => {
    saveBridgeProfiles([PROFILE]);
    const user = userEvent.setup();
    mount();
    await user.type(await screen.findByTestId("task-title-input"), "t");
    await user.type(screen.getByTestId("task-dir-input"), "/repo");
    await user.selectOptions(await screen.findByTestId("task-bridge-select"), "p1");

    await user.click(screen.getByRole("button", { name: /儲存|Save/ }));

    expect(createTask).toHaveBeenCalledTimes(1);
    const args = createTask.mock.calls[0][1];
    expect(args.use_bridge).toBe(true);
    expect(JSON.parse(args.bridge_tiers)).toEqual({
      opus: PROFILE.opus,
      sonnet: PROFILE.sonnet,
      haiku: PROFILE.haiku,
    });
  });

  it("選「走橋接（沿用目前設定）」存檔時 bridge_tiers 是 null", async () => {
    const user = userEvent.setup();
    mount();
    await user.type(await screen.findByTestId("task-title-input"), "t");
    await user.type(screen.getByTestId("task-dir-input"), "/repo");
    await user.selectOptions(await screen.findByTestId("task-bridge-select"), "current");

    await user.click(screen.getByRole("button", { name: /儲存|Save/ }));

    const args = createTask.mock.calls[0][1];
    expect(args.use_bridge).toBe(true);
    expect(args.bridge_tiers).toBeNull();
  });

  it("重新編輯：快照跟現存的帳號組合匹配時，預選該組合", async () => {
    saveBridgeProfiles([PROFILE]);
    const card = {
      ...BASE_CARD,
      use_bridge: true,
      bridge_tiers: JSON.stringify({ opus: PROFILE.opus, sonnet: null, haiku: null }),
    };
    mount(card);
    const select = await screen.findByTestId("task-bridge-select");
    expect(select).toHaveValue("p1");
  });

  it("重新編輯：快照跟任何現存組合都對不上時，顯示自訂組合、不強迫重選", async () => {
    const card = {
      ...BASE_CARD,
      use_bridge: true,
      bridge_tiers: JSON.stringify({
        opus: { provider_id: "acct-deleted", model: "m" },
        sonnet: null,
        haiku: null,
      }),
    };
    mount(card);
    const select = await screen.findByTestId("task-bridge-select");
    expect(select).toHaveValue("custom");
    expect(
      screen.getByRole("option", { name: /此卡自訂組合|custom for this card/ }),
    ).toBeInTheDocument();
  });

  it("橋接 server 沒在跑時不顯示派工方式選單", async () => {
    bridgeStatus.mockResolvedValue({ running: false, port: null, token: null, error: null });
    mount();
    await screen.findByTestId("task-title-input");
    // 沒有明確的「已載入完成」信號可等，等一輪 microtask 讓 bridgeStatus()
    // 的 .then() 有機會跑完，再斷言選單真的沒被加進去。
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByTestId("task-bridge-select")).not.toBeInTheDocument();
  });

  it("編輯舊卡片時，就算 server 沒在跑，存檔仍保留原本的橋接設定", async () => {
    bridgeStatus.mockResolvedValue({ running: false, port: null, token: null, error: null });
    const card = {
      ...BASE_CARD,
      use_bridge: true,
      bridge_tiers: JSON.stringify({ opus: { provider_id: "acct-a", model: "m-a" }, sonnet: null, haiku: null }),
    };
    const user = userEvent.setup();
    mount(card);
    await screen.findByTestId("task-title-input");
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByTestId("task-bridge-select")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /儲存|Save/ }));

    expect(updateTask).toHaveBeenCalledTimes(1);
    const args = updateTask.mock.calls[0][1];
    expect(args.use_bridge).toBe(true);
    expect(args.bridge_tiers).toBe(card.bridge_tiers);
  });
});
