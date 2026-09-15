import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { SaveHostPrompt } from "./SaveHostPrompt";
import { LocaleProvider } from "../../contexts/LocaleContext";

function renderPrompt() {
  const onSave = vi.fn();
  const onSkip = vi.fn();
  render(
    <LocaleProvider>
      <SaveHostPrompt onSave={onSave} onSkip={onSkip} />
    </LocaleProvider>,
  );
  return { onSave, onSkip };
}

describe("SaveHostPrompt", () => {
  it("按儲存會回報輸入的別名", async () => {
    const { onSave, onSkip } = renderPrompt();
    await userEvent.type(screen.getByLabelText("別名"), "辦公室 NAS");
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(onSave).toHaveBeenCalledWith("辦公室 NAS");
    expect(onSkip).not.toHaveBeenCalled();
  });

  it("沒填別名就按儲存，回報空字串讓呼叫端自己決定預設值", async () => {
    // 預設別名（目前是 host:port）由 ConnectDialog 決定，不是這個元件——
    // 在這裡偷偷填入預設值的話，呼叫端就分不出「使用者真的打了這串」和
    // 「使用者沒填」。
    const { onSave } = renderPrompt();
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(onSave).toHaveBeenCalledWith("");
  });

  it("按不用只回報略過，不回報儲存", async () => {
    const { onSave, onSkip } = renderPrompt();
    await userEvent.type(screen.getByLabelText("別名"), "打了一半");
    await userEvent.click(screen.getByRole("button", { name: "不用" }));
    expect(onSkip).toHaveBeenCalledTimes(1);
    expect(onSave).not.toHaveBeenCalled();
  });
});
