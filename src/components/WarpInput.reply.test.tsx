import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { LocaleProvider } from "../contexts/LocaleContext";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { WarpInput, type WarpReply } from "./WarpInput";

beforeEach(() => localStorage.clear());

function setup(replyOverrides: Partial<WarpReply> = {}) {
  const onSubmit = vi.fn();
  const reply: WarpReply = {
    command: "winget install x",
    prompt: null,
    onSend: vi.fn(),
    onStop: vi.fn(),
    onForceEnd: vi.fn(),
    ...replyOverrides,
  };
  render(
    <LocaleProvider>
      <WarpInput onSubmit={onSubmit} sessionId="s1" isCommandRunning onRawKey={vi.fn()} reply={reply} />
    </LocaleProvider>,
  );
  return { onSubmit, reply };
}

describe("WarpInput 回覆模式", () => {
  it("Enter 把文字送給執行中的指令，不當成新指令、不存歷史", async () => {
    const { onSubmit, reply } = setup();
    const user = userEvent.setup();
    const box = screen.getByRole("textbox");
    await user.type(box, "Y");
    await user.keyboard("{Enter}");
    expect(reply.onSend).toHaveBeenCalledWith("Y");
    expect(onSubmit).not.toHaveBeenCalled();
    expect(localStorage.getItem("aiterm-command-history")).toBeNull(); // 回覆（可能是密碼）不進歷史
    expect(box).toHaveValue("");
  });

  it("顯示提示列、停止、結束這張卡片", async () => {
    const { reply } = setup();
    expect(screen.getByText(/回覆執行中的指令：winget install x/)).toBeInTheDocument();
    fireEvent.click(screen.getByText("停止"));
    fireEvent.click(screen.getByText("結束這張卡片"));
    expect(reply.onStop).toHaveBeenCalledTimes(1);
    expect(reply.onForceEnd).toHaveBeenCalledTimes(1);
  });

  it("yes/no 提問：顯示 Y／N 快速按鈕，一鍵送出", () => {
    const { reply } = setup({ prompt: { kind: "yesno", options: ["Y", "N"] } });
    expect(screen.getByText("⏳ 等待你的輸入")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "N" }));
    expect(reply.onSend).toHaveBeenCalledWith("N");
  });

  it("按任意鍵：快速按鈕是 Enter，送空字串", () => {
    const { reply } = setup({ prompt: { kind: "anykey", options: [""] } });
    fireEvent.click(screen.getByRole("button", { name: "Enter" }));
    expect(reply.onSend).toHaveBeenCalledWith("");
  });

  it("密碼提問：輸入遮罩、沒有快速按鈕", () => {
    setup({ prompt: { kind: "password", options: [] } });
    const box = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(box).toHaveClass("warp-input-textarea--masked");
    expect(screen.queryByRole("button", { name: "Y" })).not.toBeInTheDocument();
    expect(box).toHaveAttribute("placeholder", expect.stringContaining("密碼"));
  });

  it("沒有 reply 時行為完全不變：Enter 照舊送出新指令", async () => {
    const onSubmit = vi.fn();
    render(
      <LocaleProvider>
        <WarpInput onSubmit={onSubmit} sessionId="s1" />
      </LocaleProvider>,
    );
    const user = userEvent.setup();
    await user.type(screen.getByRole("textbox"), "ls");
    await user.keyboard("{Enter}");
    expect(onSubmit).toHaveBeenCalledWith("ls");
    expect(screen.queryByText(/回覆執行中/)).not.toBeInTheDocument();
  });
});
