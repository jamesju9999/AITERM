import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeAiChat = vi.fn();
const listProviders = vi.fn();

vi.mock("../../ipc/ai", () => ({ invokeAiChat: (...a: unknown[]) => invokeAiChat(...a) }));
vi.mock("../../ipc/projects", () => ({ usedDirs: vi.fn().mockResolvedValue([]) }));
vi.mock("../../ipc/tasks", () => ({
  createTask: vi.fn(),
  updateTask: vi.fn(),
  addAttachment: vi.fn(),
  removeAttachment: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));
vi.mock("../../ipc/provider", () => ({ listProviders: (...a: unknown[]) => listProviders(...a) }));
// ModelPickerButton 會拉配額資訊，兩個都打真的 IPC。
vi.mock("../../ipc/usage", () => ({
  usageQuota: vi.fn().mockResolvedValue({ status: "not_applicable", provider_id: "x" }),
  usageQuotaAll: vi.fn().mockResolvedValue([]),
  primaryWindow: () => null,
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TaskEditorDialog } from "./TaskEditorDialog";
import type { ProviderInfo } from "../../ipc/provider";

const PROVIDERS: ProviderInfo[] = [
  {
    id: "p-a", display_name: "Provider A", provider_type: "anthropic",
    base_url: null, oauth_client_id: null, model: "claude-x",
    supports_json_mode: true, has_api_key: true, is_default: true, auth_method: null,
  },
];

const mount = () =>
  render(
    <LocaleProvider>
      <TaskEditorDialog projectId="p1" card={null} onClose={vi.fn()} onSaved={vi.fn()} />
    </LocaleProvider>,
  );

/** 在工作內容欄打字。 */
const typeBody = async (text: string) => {
  const box = screen.getByTestId("task-body-input");
  await userEvent.clear(box);
  await userEvent.type(box, text);
  return box as HTMLTextAreaElement;
};

describe("TaskEditorDialog 的 AI 潤飾", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listProviders.mockResolvedValue(PROVIDERS);
    invokeAiChat.mockResolvedValue({
      content: "標題：整理打卡 API\n---\n目標：產出 OpenAPI 規格檔。",
    });
  });

  it("潤飾後把改寫的內容寫回輸入框", async () => {
    mount();
    const box = await typeBody("把打卡 API 整理一下");

    await userEvent.click(screen.getByTestId("task-refine"));

    await waitFor(() => expect(box.value).toBe("目標：產出 OpenAPI 規格檔。"));
  });

  // 標題常常忘了填，而沒有標題就存不了（儲存鈕會 disabled）。
  it("標題是空的就一併填上", async () => {
    mount();
    await typeBody("把打卡 API 整理一下");

    await userEvent.click(screen.getByTestId("task-refine"));

    const title = screen.getByTestId("task-title-input") as HTMLInputElement;
    await waitFor(() => expect(title.value).toBe("整理打卡 API"));
  });

  // 使用者自己寫好的標題是明確的意圖，AI 不可以蓋掉。
  it("標題已經有字就不覆蓋", async () => {
    mount();
    const title = screen.getByTestId("task-title-input") as HTMLInputElement;
    await userEvent.type(title, "我自己的標題");
    await typeBody("把打卡 API 整理一下");

    await userEvent.click(screen.getByTestId("task-refine"));

    await waitFor(() => expect(invokeAiChat).toHaveBeenCalled());
    expect(title.value).toBe("我自己的標題");
  });

  it("還原把原文換回來", async () => {
    mount();
    const box = await typeBody("把打卡 API 整理一下");
    await userEvent.click(screen.getByTestId("task-refine"));
    await waitFor(() => expect(box.value).toBe("目標：產出 OpenAPI 規格檔。"));

    await userEvent.click(screen.getByTestId("task-refine-undo"));

    expect(box.value).toBe("把打卡 API 整理一下");
  });

  // 還原也要把「順便填上的標題」收回去，不然使用者按了還原卻留下一個
  // 不是自己寫的標題。
  it("還原也清掉 AI 填的標題", async () => {
    mount();
    await typeBody("把打卡 API 整理一下");
    const title = screen.getByTestId("task-title-input") as HTMLInputElement;
    await userEvent.click(screen.getByTestId("task-refine"));
    await waitFor(() => expect(title.value).toBe("整理打卡 API"));

    await userEvent.click(screen.getByTestId("task-refine-undo"));

    expect(title.value).toBe("");
  });

  it("沒有還原的東西時不顯示還原鈕", async () => {
    mount();
    await typeBody("隨便打點字");
    expect(screen.queryByTestId("task-refine-undo")).not.toBeInTheDocument();
  });

  // 空白的內容沒有東西可以潤飾，讓按鈕可按只會浪費一次呼叫。
  it("工作內容空白時潤飾鈕不能按", async () => {
    mount();
    expect(screen.getByTestId("task-refine")).toBeDisabled();
  });

  it("AI 失敗時顯示錯誤且不動輸入框", async () => {
    invokeAiChat.mockRejectedValue(new Error("boom"));
    mount();
    const box = await typeBody("原本的草稿");

    await userEvent.click(screen.getByTestId("task-refine"));

    expect(await screen.findByTestId("task-refine-error")).toBeInTheDocument();
    expect(box.value).toBe("原本的草稿");
  });
});
