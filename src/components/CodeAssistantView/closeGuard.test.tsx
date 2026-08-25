import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// 掛載 CodeAssistantView 只需要這三個 mock。刻意不設定 projectRoot，
// 讓它渲染「選擇專案目錄」的空狀態分支——註冊 guard 的 effect 一樣會跑，
// 但不會拉進 ModelPickerButton 的配額 IPC。
vi.mock("../../ipc/provider", () => ({ listProviders: () => Promise.resolve([]) }));
vi.mock("../../ipc/config", () => ({ getConfig: () => Promise.resolve({ submit_shortcut: "enter" }) }));

const fakeAssistant = {
  messages: [] as { role: string; content: string }[],
  isStreaming: false,
  error: null as string | null,
  isFallbackMode: false,
  tokenCount: 0,
  tokenLimit: 100,
  send: vi.fn(),
  clear: vi.fn(),
};
vi.mock("../../hooks/useCodeAssistant", () => ({
  useCodeAssistant: () => fakeAssistant,
}));

import { CodeAssistantView } from "./index";
import { LocaleProvider } from "../../contexts/LocaleContext";

// register/unregister 在真實的 TerminalApp 裡是 useCallback([]) 的穩定引用。
// 測試必須照樣給穩定引用，否則每次 rerender 都會讓 effect 重新註冊一輪，
// 測到的就不是真實情境。
function mountAndCaptureGuard() {
  let guard: (() => Promise<boolean>) | undefined;
  const register = (_id: string, g: () => Promise<boolean>) => { guard = g; };
  const unregister = vi.fn();
  // register/unregister 是穩定引用，但每次呼叫都要回傳「新的」React element——
  // React 對完全相同的 element 參照（連內層都沒變）會直接 bail out、不重新
  // render 子樹。用 renderUi() 讓 rerender 真的走一次 CodeAssistantView 的
  // render body，才讀得到 fakeAssistant.messages 的最新值。
  const renderUi = () => (
    <LocaleProvider>
      <CodeAssistantView
        isActive
        tabId="tab-1"
        registerCloseGuard={register}
        unregisterCloseGuard={unregister}
      />
    </LocaleProvider>
  );
  const view = render(renderUi());
  if (!guard) throw new Error("CodeAssistantView 沒有註冊 close guard");
  return { guard, view, unregister, renderUi };
}

beforeEach(() => {
  fakeAssistant.messages = [];
  fakeAssistant.isStreaming = false;
});

describe("Agent 分頁 close guard", () => {
  it("全新空白分頁：直接放行且不跳確認框", async () => {
    const { guard } = mountAndCaptureGuard();
    await expect(guard()).resolves.toBe(true);
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
  });

  it("已有對話紀錄：跳確認框，Promise 保持未定", async () => {
    fakeAssistant.messages = [{ role: "user", content: "hi" }];
    const { guard } = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("對話尚未保存");
    expect(settled).toBe("pending");
  });

  it("正在串流：確認框顯示串流版本的文案", async () => {
    fakeAssistant.isStreaming = true;
    const { guard } = mountAndCaptureGuard();

    await act(async () => { void guard(); });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("AI 正在回應中");
  });

  it("按「關閉並捨棄」：resolve true", async () => {
    fakeAssistant.messages = [{ role: "user", content: "hi" }];
    const { guard } = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });
    await userEvent.click(screen.getByRole("button", { name: "關閉並捨棄" }));

    expect(settled).toBe(true);
  });

  it("按「取消（返回對話）」：resolve false 且確認框消失", async () => {
    fakeAssistant.messages = [{ role: "user", content: "hi" }];
    const { guard } = mountAndCaptureGuard();

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });
    await userEvent.click(screen.getByRole("button", { name: "取消（返回對話）" }));

    expect(settled).toBe(false);
    expect(screen.queryByRole("heading", { level: 3 })).not.toBeInTheDocument();
  });

  // 這一題釘住整個功能最容易靜默失效的地方：guard 是在 messages 還是空的
  // 時候註冊的，如果它閉包捕捉了當時的 messages，之後談再多輪也會直接放行。
  it("註冊之後才產生的對話，guard 仍看得到（不可讀到過期狀態）", async () => {
    const { guard, view, renderUi } = mountAndCaptureGuard();

    // 註冊當下 messages 是空的。模擬「註冊完 guard 之後，使用者才跟 AI 談話」，
    // 用同一組穩定 props（register/unregister 引用不變）重繪，讓元件讀到新的 messages。
    fakeAssistant.messages = [{ role: "user", content: "後來才講的話" }];
    view.rerender(renderUi());

    let settled: unknown = "pending";
    await act(async () => { void guard().then((v) => { settled = v; }); });

    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("對話尚未保存");
    expect(settled).toBe("pending");
  });

  it("unmount 時解除註冊", () => {
    const { view, unregister } = mountAndCaptureGuard();
    view.unmount();
    expect(unregister).toHaveBeenCalledWith("tab-1");
  });
});
