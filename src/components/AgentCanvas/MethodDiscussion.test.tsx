import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { MethodDiscussion } from "./MethodDiscussion";

const decisionReply =
  "建議批次。\n```decision\n" +
  JSON.stringify({ approach: "batch", reason: "骨架相同", trial: "先試 5 支", requirement: "轉換 Cm 畫面" }) +
  "\n```";

function setup(complete: (m: { role: "user"; content: string }[], s: string) => Promise<string>) {
  const onConfirm = vi.fn();
  const onBack = vi.fn();
  render(
    <LocaleProvider>
      <MethodDiscussion
        complete={complete}
        getContext={async () => ({ brief: "", refs: [] })}
        busy={false}
        onConfirm={onConfirm}
        onBack={onBack}
      />
    </LocaleProvider>,
  );
  return { onConfirm, onBack };
}

const send = async (text: string) => {
  fireEvent.change(screen.getByTestId("ac-md-input"), { target: { value: text } });
  fireEvent.click(screen.getByTestId("ac-md-send"));
};

describe("MethodDiscussion", () => {
  it("keeps confirm disabled until the AI proposes a decision", async () => {
    const complete = vi.fn().mockResolvedValue("有幾支？");
    setup(complete);
    await send("要轉換舊平台");
    await waitFor(() => expect(screen.getByText("有幾支？")).toBeTruthy());
    expect((screen.getByTestId("ac-md-confirm") as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows the proposal and confirms with it", async () => {
    const complete = vi.fn().mockResolvedValue(decisionReply);
    const { onConfirm } = setup(complete);
    await send("533 支畫面");
    await waitFor(() => expect(screen.getByTestId("ac-md-decision")).toBeTruthy());
    const confirm = screen.getByTestId("ac-md-confirm") as HTMLButtonElement;
    expect(confirm.disabled).toBe(false);
    fireEvent.click(confirm);
    expect(onConfirm).toHaveBeenCalledWith(expect.objectContaining({ approach: "batch", requirement: "轉換 Cm 畫面" }));
  });

  it("does not show the raw decision block in the chat", async () => {
    setup(vi.fn().mockResolvedValue(decisionReply));
    await send("533 支畫面");
    await waitFor(() => expect(screen.getByTestId("ac-md-decision")).toBeTruthy());
    expect(screen.queryByText(/```decision/)).toBeNull();
  });

  it("sends the whole history on the second turn", async () => {
    const complete = vi.fn().mockResolvedValueOnce("有幾支？").mockResolvedValueOnce("好");
    setup(complete);
    await send("要轉換舊平台");
    await waitFor(() => expect(screen.getByText("有幾支？")).toBeTruthy());
    await send("533 支");
    await waitFor(() => expect(complete).toHaveBeenCalledTimes(2));
    const second = complete.mock.calls[1][0][0].content as string;
    expect(second).toContain("要轉換舊平台");
    expect(second).toContain("有幾支？");
    expect(second).toContain("533 支");
  });

  it("shows the error when the AI call fails", async () => {
    setup(vi.fn().mockRejectedValue(new Error("boom")));
    await send("hi");
    await waitFor(() => expect(screen.getByText(/boom/)).toBeTruthy());
  });

  it("the back button calls onBack", () => {
    const { onBack } = setup(vi.fn());
    fireEvent.click(screen.getByTestId("ac-md-back"));
    expect(onBack).toHaveBeenCalled();
  });

  it("keeps the previous proposal when a later reply has none", async () => {
    const complete = vi.fn().mockResolvedValueOnce(decisionReply).mockResolvedValueOnce("還有什麼要補充？");
    setup(complete);
    await send("533 支畫面");
    await waitFor(() => expect(screen.getByTestId("ac-md-decision")).toBeTruthy());
    await send("對了還有權限頁");
    await waitFor(() => expect(complete).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.getByText("還有什麼要補充？")).toBeTruthy());
    expect(screen.getByTestId("ac-md-decision")).toBeTruthy();
  });
});
