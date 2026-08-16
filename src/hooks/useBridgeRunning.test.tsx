import { renderHook, waitFor, fireEvent, screen } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { MemoryRouter, useNavigate } from "react-router-dom";
import { createElement, type ReactNode } from "react";

vi.mock("../ipc/bridge", () => ({ bridgeStatus: vi.fn() }));

import { useBridgeRunning } from "./useBridgeRunning";
import { bridgeStatus } from "../ipc/bridge";

beforeEach(() => {
  vi.mocked(bridgeStatus).mockReset();
});

// 掛一顆隱藏按鈕，讓測試能觸發 pathname 切換，同時渲染被測 hook 所在的
// children——模擬 TerminalApp 永遠掛著、只有 pathname 在變的真實情境。
function NavButtons() {
  const navigate = useNavigate();
  return (
    <>
      <button onClick={() => navigate("/settings")}>go-settings</button>
      <button onClick={() => navigate("/")}>go-home</button>
    </>
  );
}

function renderAtPath(initialPath: string, children: ReactNode) {
  return renderHook(() => useBridgeRunning(), {
    wrapper: ({ children: hookChildren }) =>
      createElement(
        MemoryRouter,
        { initialEntries: [initialPath] },
        createElement(NavButtons, null),
        hookChildren,
        children,
      ),
  });
}

describe("useBridgeRunning", () => {
  it("在首頁路徑（/）掛載時查一次橋接狀態", async () => {
    vi.mocked(bridgeStatus).mockResolvedValue({ running: true, port: 8317, token: "tok", error: null });
    const { result } = renderAtPath("/", null);
    await waitFor(() => expect(result.current).toBe(true));
  });

  // 這是回歸測試的核心：TerminalApp 不會 unmount LaunchGrid，所以「回到首頁」
  // 必須靠 pathname 變化觸發重查，不能只在 mount 時查一次。
  it("pathname 回到 / 時重新查詢（不需要重新掛載）", async () => {
    vi.mocked(bridgeStatus).mockResolvedValue({ running: false, port: null, token: null, error: null });
    const { result } = renderAtPath("/", null);
    await waitFor(() => expect(result.current).toBe(false));
    expect(bridgeStatus).toHaveBeenCalledTimes(1);

    vi.mocked(bridgeStatus).mockResolvedValue({ running: true, port: 8317, token: "tok", error: null });
    fireEvent.click(screen.getByText("go-settings"));
    fireEvent.click(screen.getByText("go-home"));

    await waitFor(() => expect(result.current).toBe(true));
    expect(bridgeStatus).toHaveBeenCalledTimes(2);
  });

  it("停在 /settings 時不查（那不是這個 hook 的使用場景）", () => {
    renderAtPath("/settings", null);
    expect(bridgeStatus).not.toHaveBeenCalled();
  });
});
