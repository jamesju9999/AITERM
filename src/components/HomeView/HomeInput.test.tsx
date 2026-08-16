import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const invokeAiChatMock = vi.fn();
vi.mock("../../ipc/ai", () => ({
  invokeAiChat: (...args: unknown[]) => invokeAiChatMock(...args),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { HomeInput } from "./HomeInput";

function renderInput(onRoute = vi.fn()) {
  render(
    <LocaleProvider>
      <HomeInput onRoute={onRoute} />
    </LocaleProvider>,
  );
  return onRoute;
}

beforeEach(() => {
  invokeAiChatMock.mockReset();
});

function submit(text: string) {
  const box = screen.getByRole("textbox");
  fireEvent.change(box, { target: { value: text } });
  fireEvent.keyDown(box, { key: "Enter" });
}

describe("HomeInput", () => {
  // 掛載時不可以打 AI：首頁每次顯示都重新掛載，那會變成一顆自動打點的按鈕。
  it("掛載時不呼叫 AI", () => {
    renderInput();
    expect(invokeAiChatMock).not.toHaveBeenCalled();
  });

  it("AI 判斷是資料庫就開資料庫分頁", async () => {
    invokeAiChatMock.mockResolvedValue({ content: '{"type":"database"}' });
    const onRoute = renderInput();
    submit("查一下訂單表");
    await waitFor(() =>
      expect(onRoute).toHaveBeenCalledWith(
        expect.objectContaining({ type: "database", fallback: false }),
      ),
    );
  });

  // 降級路徑：AI 掛掉不能讓輸入框變成死的。
  it("AI 失敗時降級為終端機 + agent 任務", async () => {
    invokeAiChatMock.mockRejectedValue({ kind: "not_configured" });
    const onRoute = renderInput();
    submit("幫我修 build");
    await waitFor(() =>
      expect(onRoute).toHaveBeenCalledWith(
        expect.objectContaining({ type: "terminal", mission: "幫我修 build", fallback: true }),
      ),
    );
  });

  // content 為 null 是既有型別允許的（工具呼叫時），不能因此炸掉。
  it("回應的 content 是 null 時降級", async () => {
    invokeAiChatMock.mockResolvedValue({ content: null });
    const onRoute = renderInput();
    submit("做點事");
    await waitFor(() =>
      expect(onRoute).toHaveBeenCalledWith(expect.objectContaining({ fallback: true })),
    );
  });

  it("空白輸入不送出", () => {
    const onRoute = renderInput();
    submit("   ");
    expect(invokeAiChatMock).not.toHaveBeenCalled();
    expect(onRoute).not.toHaveBeenCalled();
  });

  // 判斷中重複按 Enter 會開出好幾個分頁。
  it("判斷中不接受重複送出", () => {
    invokeAiChatMock.mockImplementation(() => new Promise(() => {}));
    renderInput();
    submit("做事");
    submit("做事");
    expect(invokeAiChatMock).toHaveBeenCalledTimes(1);
  });

  // 送出的提示詞必須帶上可選的分頁類型，否則 AI 只能亂猜。
  it("提示詞裡包含可路由的分頁類型", async () => {
    invokeAiChatMock.mockResolvedValue({ content: '{"type":"terminal"}' });
    renderInput();
    submit("做點事");
    await waitFor(() => expect(invokeAiChatMock).toHaveBeenCalled());
    const prompt = JSON.stringify(invokeAiChatMock.mock.calls[0][0]);
    expect(prompt).toContain("terminal");
    expect(prompt).toContain("database");
    expect(prompt).toContain("knowledge-base");
  });

  // hidden 的類型（api-docs / mail）入口是刻意收起來的，
  // 提示詞列出它們等於邀請 AI 路由過去。
  it("提示詞不含 hidden 的分頁類型", async () => {
    invokeAiChatMock.mockResolvedValue({ content: '{"type":"terminal"}' });
    renderInput();
    submit("做點事");
    await waitFor(() => expect(invokeAiChatMock).toHaveBeenCalled());
    const prompt = JSON.stringify(invokeAiChatMock.mock.calls[0][0]);
    expect(prompt).not.toContain("api-docs");
    expect(prompt).not.toContain("mail");
  });
});
