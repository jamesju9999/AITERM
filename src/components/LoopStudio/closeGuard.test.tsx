import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// 掛載 LoopStudio 只需要這兩個 mock：listProviders 是唯一在 mount 時
// 被呼叫的 IPC，其餘都是事件驅動的。
vi.mock("../../ipc/provider", () => ({ listProviders: () => Promise.resolve([]) }));

const fakeLoop = {
  trace: [] as unknown[],
  isRunning: false,
  iteration: 0,
  start: vi.fn(),
  stop: vi.fn(),
  resume: vi.fn(),
  pendingConfirmation: null as unknown,
};
vi.mock("../../hooks/useOrchestratorLoop", () => ({
  useOrchestratorLoop: () => fakeLoop,
}));

import { LoopStudioView } from "./index";
import { LocaleProvider } from "../../contexts/LocaleContext";

/** 掛載元件，回傳被註冊的 close guard。 */
function mountAndCaptureGuard() {
  let guard: (() => Promise<boolean>) | undefined;
  render(
    <LocaleProvider>
      <LoopStudioView
        tabId="tab-1"
        registerCloseGuard={(_id, g) => { guard = g; }}
        unregisterCloseGuard={vi.fn()}
      />
    </LocaleProvider>
  );
  if (!guard) throw new Error("LoopStudio 沒有註冊 close guard");
  return guard;
}

beforeEach(() => {
  fakeLoop.isRunning = false;
  fakeLoop.stop.mockClear();
});

describe("LoopStudio close guard", () => {
  it("Loop 執行中：跳確認框，Promise 保持未定", async () => {
    fakeLoop.isRunning = true;
    const guard = mountAndCaptureGuard();

    let settled: unknown = "pending";
    // guard() 內部會 setState，必須包在 act 裡才會 flush 出 Modal。
    await act(async () => { void guard().then((v) => { settled = v; }); });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("Loop 正在執行中");
    expect(settled).toBe("pending");
  });

  it("乾淨狀態：直接放行且不跳確認框", async () => {
    const guard = mountAndCaptureGuard();
    await expect(guard()).resolves.toBe(true);
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
  });

  it("執行中按「關閉不儲存」：resolve true 並停止 loop", async () => {
    fakeLoop.isRunning = true;
    const guard = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });
    await userEvent.click(screen.getByRole("button", { name: "關閉不儲存" }));

    expect(settled).toBe(true);
    expect(fakeLoop.stop).toHaveBeenCalledTimes(1);
  });

  it("執行中按「取消」：resolve false 且不停止 loop", async () => {
    fakeLoop.isRunning = true;
    const guard = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });
    await userEvent.click(screen.getByRole("button", { name: "取消（繼續編輯）" }));

    expect(settled).toBe(false);
    expect(fakeLoop.stop).not.toHaveBeenCalled();
  });
});
