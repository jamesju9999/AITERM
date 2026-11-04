import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { NudgeBox } from "./NudgeBox";

const renderBox = (zoom: number, onSend = vi.fn().mockResolvedValue(undefined)) => {
  render(
    <LocaleProvider>
      <div data-testid="outside">外面</div>
      <div data-testid="scaled">
        <NudgeBox agentId="a1" zoom={zoom} onSend={onSend} />
      </div>
    </LocaleProvider>,
  );
  return onSend;
};

describe("NudgeBox 縮放比例", () => {
  it("縮放 ≥ 75%：維持行內輸入，沒有精簡按鈕", () => {
    renderBox(0.75);
    expect(screen.getByTestId("nudge-input-a1")).toBeInTheDocument();
    expect(screen.queryByTestId("nudge-open-a1")).toBeNull();
  });

  it("縮放 < 75%：只顯示一顆「補充」按鈕，不顯示行內輸入", () => {
    renderBox(0.45);
    expect(screen.getByTestId("nudge-open-a1")).toHaveTextContent("補充");
    expect(screen.queryByTestId("nudge-input-a1")).toBeNull();
  });

  it("按下去開浮出輸入框：掛在 body、不在被縮放的節點裡，position: fixed", () => {
    renderBox(0.45);
    fireEvent.click(screen.getByTestId("nudge-open-a1"));
    const pop = screen.getByTestId("nudge-pop-a1");
    expect(document.body.contains(pop)).toBe(true);
    expect(screen.getByTestId("scaled").contains(pop)).toBe(false);
    expect(pop.style.position).toBe("fixed");
    expect(screen.getByTestId("nudge-input-a1")).toBeInTheDocument();
  });

  it("送出會呼叫 onSend（去頭尾空白）並關閉浮出框", async () => {
    const onSend = renderBox(0.45);
    fireEvent.click(screen.getByTestId("nudge-open-a1"));
    fireEvent.change(screen.getByTestId("nudge-input-a1"), { target: { value: "  請改用繁體中文  " } });
    fireEvent.click(screen.getByTestId("nudge-send-a1"));
    await waitFor(() => expect(onSend).toHaveBeenCalledWith("請改用繁體中文"));
    await waitFor(() => expect(screen.queryByTestId("nudge-pop-a1")).toBeNull());
  });

  it("送出失敗：浮出框維持開啟並保留輸入", async () => {
    const onSend = vi.fn().mockRejectedValue(new Error("卡片剛好結束"));
    renderBox(0.45, onSend);
    fireEvent.click(screen.getByTestId("nudge-open-a1"));
    fireEvent.change(screen.getByTestId("nudge-input-a1"), { target: { value: "再試一次" } });
    fireEvent.click(screen.getByTestId("nudge-send-a1"));
    await waitFor(() => expect(onSend).toHaveBeenCalled());
    // busy 結束（輸入框重新可用）後才檢查，確認不是「還在送」的假象。
    await waitFor(() => expect(screen.getByTestId("nudge-input-a1")).not.toBeDisabled());
    expect(screen.getByTestId("nudge-pop-a1")).toBeInTheDocument();
    expect((screen.getByTestId("nudge-input-a1") as HTMLInputElement).value).toBe("再試一次");
  });

  it("Esc 與點外面都會關閉", () => {
    renderBox(0.45);
    fireEvent.click(screen.getByTestId("nudge-open-a1"));
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByTestId("nudge-pop-a1")).toBeNull();

    fireEvent.click(screen.getByTestId("nudge-open-a1"));
    fireEvent.mouseDown(screen.getByTestId("outside"));
    expect(screen.queryByTestId("nudge-pop-a1")).toBeNull();
  });

  it("點浮出框裡面不會關閉", () => {
    renderBox(0.45);
    fireEvent.click(screen.getByTestId("nudge-open-a1"));
    fireEvent.mouseDown(screen.getByTestId("nudge-input-a1"));
    expect(screen.getByTestId("nudge-pop-a1")).toBeInTheDocument();
  });

  it("輸入法組字中按 Enter 不送出；組字結束、過了保護窗後 Enter 才送", async () => {
    const onSend = renderBox(0.45);
    fireEvent.click(screen.getByTestId("nudge-open-a1"));
    const input = screen.getByTestId("nudge-input-a1");
    fireEvent.change(input, { target: { value: "你好" } });
    fireEvent.compositionStart(input);
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onSend).not.toHaveBeenCalled();
    fireEvent.compositionEnd(input);
    await new Promise((r) => setTimeout(r, 150));
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(onSend).toHaveBeenCalledWith("你好"));
  });
});
