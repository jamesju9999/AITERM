import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const startMock = vi.fn();
const stopMock = vi.fn();
const kickMock = vi.fn();
const revokeMock = vi.fn();
let hookState = {
  sharing: false,
  code: null as string | null,
  port: null as number | null,
  viewers: [] as Array<{ viewerId: string; displayName: string; mode: string }>,
};

vi.mock("../../hooks/useShareHost", () => ({
  useShareHost: () => ({
    ...hookState,
    start: startMock,
    stop: stopMock,
    kick: kickMock,
    revokeControl: revokeMock,
  }),
}));

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await import("../../lib/i18n");
  return { useLocale: () => ({ t: translations["zh-TW"], locale: "zh-TW", setLocale: () => {} }) };
});

import { SharePanel } from "./index";

beforeEach(() => {
  startMock.mockReset().mockResolvedValue(undefined);
  stopMock.mockReset().mockResolvedValue(undefined);
  kickMock.mockReset().mockResolvedValue(undefined);
  revokeMock.mockReset().mockResolvedValue(undefined);
  hookState = { sharing: false, code: null, port: null, viewers: [] };
});

describe("SharePanel", () => {
  it("starts sharing when the button is pressed", async () => {
    render(<SharePanel sessionId="t1" />);
    await userEvent.click(screen.getByRole("button", { name: /分享/ }));
    expect(startMock).toHaveBeenCalled();
  });

  it("shows the code and the address together", async () => {
    // 兩個都要顯示：對方自動發現失敗時，唸位址就好，不用回頭找。
    hookState = { sharing: true, code: "559207", port: 47823, viewers: [] };
    render(<SharePanel sessionId="t1" />);
    await userEvent.click(screen.getByRole("button", { name: /分享/ }));

    expect(await screen.findByText("559207")).toBeInTheDocument();
    expect(screen.getByText(/47823/)).toBeInTheDocument();
  });

  it("warns about the firewall prompt before it appears", async () => {
    // 使用者被系統彈窗嚇到而反射性按拒絕，是這個功能最可惜的失敗方式。
    hookState = { sharing: true, code: "559207", port: 47823, viewers: [] };
    render(<SharePanel sessionId="t1" />);
    await userEvent.click(screen.getByRole("button", { name: /分享/ }));

    expect(await screen.findByText(/允許連入連線/)).toBeInTheDocument();
  });

  it("lists viewers with their mode and lets the host disconnect them", async () => {
    hookState = {
      sharing: true,
      code: "559207",
      port: 47823,
      viewers: [{ viewerId: "v1", displayName: "Alice", mode: "control" }],
    };
    render(<SharePanel sessionId="t1" />);
    await userEvent.click(screen.getByRole("button", { name: /分享/ }));

    expect(await screen.findByText("Alice")).toBeInTheDocument();
    expect(screen.getByText(/控制中/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: /中斷連線/ }));
    expect(kickMock).toHaveBeenCalledWith("v1");
  });

  it("never displays a verification code", async () => {
    // 主控端的 4 位碼根本不會到前端——同意視窗要使用者輸入對方唸的碼。
    // 這個測試守著「不要哪天為了『方便』把碼加進面板」。
    hookState = {
      sharing: true,
      code: "559207",
      port: 47823,
      viewers: [{ viewerId: "v1", displayName: "Alice", mode: "read_only" }],
    };
    const { container } = render(<SharePanel sessionId="t1" />);
    await userEvent.click(screen.getByRole("button", { name: /分享/ }));

    // 6 位短碼是要給對方輸入的，可以顯示；4 位驗證碼不行。
    await waitFor(() => expect(screen.getByText("559207")).toBeInTheDocument());
    expect(container.textContent).not.toMatch(/\b\d{4}\b(?!\d)/);
  });
});

/**
 * 這條守的是**呼叫點**，不是元件本身。
 *
 * 實機測試時觀看端一連上就顯示「那個終端機已經關閉」，原因是 TerminalView
 * 把 React 的分頁 id 傳給了 SharePanel，而後端拿這個值去 `PtyManager` 查
 * 串流——查不到就回 `SessionClosed`。
 *
 * **整套 Rust 測試都抓不到這個 bug**：測試裡是 `let tab_id = pty.create(...)`，
 * 直接把 PTY id 當分頁 id 用，兩者在測試中根本是同一個值。只有掃真實呼叫
 * 點的測試能守住這件事。
 */
describe("SharePanel call site", () => {
  it("is given the PTY session id, not the React tab id", async () => {
    const src = (await import("../TerminalView.tsx?raw")).default;
    expect(src).toContain("<SharePanel sessionId={sessionId} />");
    expect(src).not.toMatch(/<SharePanel[^>]*\btabId\b/);
  });
});
