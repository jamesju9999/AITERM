import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { CloseConfirmDialog } from "./index";

describe("CloseConfirmDialog", () => {
  it("顯示傳入的標題、內文與兩個按鈕文字", () => {
    render(
      <CloseConfirmDialog
        title="標題在這"
        body={<>內文在這</>}
        confirmLabel="關掉"
        cancelLabel="不要"
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />
    );
    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("標題在這");
    expect(screen.getByText("內文在這")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "關掉" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "不要" })).toBeInTheDocument();
  });

  it("按確認只呼叫 onConfirm 一次，且不呼叫 onCancel", async () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(
      <CloseConfirmDialog
        title="t" body={<>b</>} confirmLabel="關掉" cancelLabel="不要"
        onConfirm={onConfirm} onCancel={onCancel}
      />
    );
    await userEvent.click(screen.getByRole("button", { name: "關掉" }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("按取消只呼叫 onCancel 一次，且不呼叫 onConfirm", async () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(
      <CloseConfirmDialog
        title="t" body={<>b</>} confirmLabel="關掉" cancelLabel="不要"
        onConfirm={onConfirm} onCancel={onCancel}
      />
    );
    await userEvent.click(screen.getByRole("button", { name: "不要" }));
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onConfirm).not.toHaveBeenCalled();
  });
});
