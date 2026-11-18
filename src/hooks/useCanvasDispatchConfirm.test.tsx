import { render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../contexts/LocaleContext";
import type { CanvasConfirmRequest } from "../ipc/canvas";
import { translations } from "../lib/i18n";
import { useCanvasDispatchConfirm } from "./useCanvasDispatchConfirm";

const confirmMock = vi.fn();
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: (...a: unknown[]) => confirmMock(...a) }));
vi.mock("../lib/webviewFocus", () => ({ restoreWebviewFocus: () => Promise.resolve() }));

let handler: (r: CanvasConfirmRequest) => void = () => {};
const decide = vi.fn(() => Promise.resolve(true));
vi.mock("../ipc/canvas", () => ({
  onCanvasConfirmDispatch: (cb: (r: CanvasConfirmRequest) => void) => {
    handler = cb;
    return Promise.resolve(() => {});
  },
  canvasConfirmDecide: (...a: unknown[]) => decide(...(a as [])),
}));

const t = translations["zh-TW"];
const req: CanvasConfirmRequest = { id: "r1", project: "請假專案", agent: "小明", agent_kind: "coder", title: "查特休", instruction: "去請假系統查剩餘特休" };

function Host() {
  useCanvasDispatchConfirm();
  return null;
}
const mount = () =>
  render(
    <LocaleProvider>
      <Host />
    </LocaleProvider>,
  );

beforeEach(() => {
  confirmMock.mockReset();
  decide.mockClear();
});

describe("useCanvasDispatchConfirm", () => {
  it("shows the project, role, title and full instruction, and sends approval when the user presses Dispatch", async () => {
    confirmMock.mockResolvedValue(true);
    mount();
    handler(req);
    await waitFor(() => expect(decide).toHaveBeenCalledWith("r1", true));
    const [message, opts] = confirmMock.mock.calls[0] as [string, { title: string; okLabel: string }];
    expect(message).toContain("小明");
    expect(message).toContain("請假專案");
    expect(message).toContain("查特休");
    expect(message).toContain("去請假系統查剩餘特休");
    expect(opts.title).toBe(t.canvas_confirm_title);
    expect(opts.okLabel).toBe(t.canvas_confirm_ok);
  });

  it("sends a rejection when the user cancels", async () => {
    confirmMock.mockResolvedValue(false);
    mount();
    handler(req);
    await waitFor(() => expect(decide).toHaveBeenCalledWith("r1", false));
  });

  it("treats a dialog error as a rejection, never as approval", async () => {
    confirmMock.mockRejectedValue(new Error("dialog failed"));
    mount();
    handler(req);
    await waitFor(() => expect(decide).toHaveBeenCalledWith("r1", false));
  });
});
