import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

const live = vi.hoisted(() => ({
  onMessage: null as null | ((m: unknown) => void),
  onEnd: null as null | (() => void),
  sent: [] as unknown[],
  closed: 0,
  opened: [] as unknown[],
  failWith: null as null | string,
}));
vi.mock("../../ipc/serverLive", () => ({
  openLive: vi.fn(async (target: unknown, onMessage: (m: unknown) => void, onEnd: () => void) => {
    live.opened.push(target);
    if (live.failWith) throw live.failWith;
    live.onMessage = onMessage;
    live.onEnd = onEnd;
    return { send: (m: unknown) => live.sent.push(m), close: () => (live.closed += 1) };
  }),
  cancelRemoteLogin: vi.fn().mockResolvedValue(undefined),
}));

import { cancelRemoteLogin } from "../../ipc/serverLive";
import { LiveViewDialog, modifiers, normalized, type LiveTarget } from "./LiveViewDialog";

const agent: LiveTarget = { kind: "agent", systemId: "s1", systemName: "ERP", cardId: "c1" };
const login: LiveTarget = { kind: "login", systemId: "s1", systemName: "ERP" };

const mount = (target: LiveTarget, props: { onClose?: () => void; onLoginDone?: () => void } = {}) => {
  const onClose = props.onClose ?? vi.fn();
  render(
    <LocaleProvider>
      <LiveViewDialog target={target} onClose={onClose} onLoginDone={props.onLoginDone} />
    </LocaleProvider>,
  );
  return { onClose };
};

const push = async (m: unknown) => {
  await waitFor(() => expect(live.onMessage).not.toBeNull());
  act(() => live.onMessage?.(m));
};
const state = (over: Record<string, unknown> = {}) => ({ t: "state", manual: false, url: "https://erp.example.com/", can_operate: true, mode: "agent", ...over });
const frame = { t: "frame", data: "QUJD", w: 1000, h: 500 };

/** 讓畫面影像有固定的位置與大小（jsdom 沒有排版）。 */
const sizeFrame = async () => {
  const img = await screen.findByTestId("live-frame");
  img.getBoundingClientRect = () => ({ left: 100, top: 50, width: 400, height: 200, right: 500, bottom: 250, x: 100, y: 50, toJSON: () => ({}) });
  return img;
};

beforeEach(() => {
  live.onMessage = null;
  live.onEnd = null;
  live.sent = [];
  live.closed = 0;
  live.opened = [];
  live.failWith = null;
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("純函式", () => {
  it("座標換成 0～1 並夾回邊界；修飾鍵對應 CDP 的位元", () => {
    const r = { left: 100, top: 50, width: 400, height: 200 };
    expect(normalized(300, 150, r)).toEqual({ x: 0.5, y: 0.5 });
    expect(normalized(0, 999, r)).toEqual({ x: 0, y: 1 });
    expect(normalized(NaN, 100, r).x).toBe(0);
    expect(modifiers({ altKey: true, ctrlKey: true, metaKey: false, shiftKey: true })).toBe(1 | 2 | 8);
  });
});

describe("LiveViewDialog：Agent 執行中", () => {
  it("連線中 → 收到畫面與狀態後顯示；開啟時帶著卡片與系統", async () => {
    mount(agent);
    expect(screen.getByTestId("live-status")).toHaveTextContent("連線中");
    await push(state({ can_operate: false }));
    await push(frame);
    expect(await screen.findByTestId("live-frame")).toHaveAttribute("src", "data:image/jpeg;base64,QUJD");
    expect(live.opened).toEqual([{ kind: "agent", systemId: "s1", cardId: "c1" }]);
  });

  it("成員：只能看，沒有按鈕，滑鼠鍵盤都不送出", async () => {
    mount(agent);
    await push(state({ can_operate: false }));
    await push(frame);
    const img = await sizeFrame();
    expect(screen.getByTestId("live-status")).toHaveTextContent("唯讀");
    expect(screen.queryByRole("button", { name: "接手" })).toBeNull();
    fireEvent.pointerDown(img, { clientX: 200, clientY: 100, button: 0 });
    fireEvent.keyDown(img.parentElement!, { key: "a", code: "KeyA", keyCode: 65 });
    expect(live.sent).toEqual([]);
  });

  it("管理員：沒接手前不送輸入；按「接手」後狀態變接手中，滑鼠點擊送正規化座標、鍵盤與貼上送對應訊息", async () => {
    mount(agent);
    await push(state());
    await push(frame);
    const img = await sizeFrame();
    expect(screen.getByTestId("live-status")).toHaveTextContent("按「接手」後才能操作");
    fireEvent.pointerDown(img, { clientX: 200, clientY: 100, button: 0 });
    expect(live.sent).toEqual([]);
    fireEvent.click(screen.getByRole("button", { name: "接手" }));
    expect(live.sent).toEqual([{ t: "takeover", on: true }]);

    await push(state({ manual: true }));
    expect(screen.getByTestId("live-status")).toHaveTextContent("你接手中");
    expect(screen.getByRole("button", { name: "交還給 Agent" })).toBeInTheDocument();
    live.sent = [];
    fireEvent.pointerDown(img, { clientX: 200, clientY: 100, button: 0 });
    fireEvent.pointerUp(img, { clientX: 200, clientY: 100, button: 2 });
    expect(live.sent).toEqual([
      { t: "mouse", type: "down", x: 0.25, y: 0.25, button: "left" },
      { t: "mouse", type: "up", x: 0.25, y: 0.25, button: "right" },
    ]);
    live.sent = [];
    const screenEl = img.parentElement!;
    fireEvent.keyDown(screenEl, { key: "a", code: "KeyA", keyCode: 65 });
    fireEvent.keyDown(screenEl, { key: "Enter", code: "Enter", keyCode: 13 });
    fireEvent.keyDown(screenEl, { key: "ArrowLeft", code: "ArrowLeft", keyCode: 37, shiftKey: true });
    fireEvent.keyUp(screenEl, { key: "a", code: "KeyA", keyCode: 65 });
    expect(live.sent).toEqual([
      { t: "key", type: "down", key: "a", code: "KeyA", text: "a", vk: 65, mods: 0 },
      { t: "key", type: "down", key: "Enter", code: "Enter", text: "\r", vk: 13, mods: 0 },
      { t: "key", type: "down", key: "ArrowLeft", code: "ArrowLeft", text: undefined, vk: 37, mods: 8 },
      { t: "key", type: "up", key: "a", code: "KeyA", text: undefined, vk: 65, mods: 0 },
    ]);
    live.sent = [];
    fireEvent.paste(screenEl, { clipboardData: { getData: () => "p@ss字" } });
    fireEvent.compositionStart(screenEl);
    fireEvent.keyDown(screenEl, { key: "n", code: "KeyN", keyCode: 229 });
    fireEvent.compositionEnd(screenEl, { data: "你好" });
    expect(live.sent).toEqual([{ t: "text", text: "p@ss字" }, { t: "text", text: "你好" }]);
    live.sent = [];
    fireEvent.click(screen.getByRole("button", { name: "交還給 Agent" }));
    expect(live.sent).toEqual([{ t: "takeover", on: false }]);
  });

  it("server 回報錯誤會顯示一下；連線結束後顯示原因並停止輸入", async () => {
    mount(agent);
    await push(state({ manual: true }));
    await push(frame);
    const img = await sizeFrame();
    await push({ t: "error", message: "請先按「接手」再操作" });
    expect(await screen.findByRole("alert")).toHaveTextContent("請先按「接手」");
    await push({ t: "closed", why: "這張卡已經結束" });
    expect(screen.getByTestId("live-status")).toHaveTextContent("這張卡已經結束");
    live.sent = [];
    fireEvent.pointerDown(img, { clientX: 200, clientY: 100, button: 0 });
    expect(live.sent).toEqual([]);
    expect(screen.queryByRole("button", { name: "交還給 Agent" })).toBeNull();
  });

  it("開啟失敗顯示原因；關閉對話框會中斷連線", async () => {
    live.failWith = "找不到這個畫面（瀏覽器可能已經結束了）";
    mount(agent);
    expect(await screen.findByTestId("live-status")).toHaveTextContent("找不到這個畫面");
    cleanup();
    live.failWith = null;
    mount(agent);
    await push(state());
    cleanup();
    expect(live.closed).toBe(1);
  });
});

describe("LiveViewDialog：遠端登入", () => {
  it("明講帳密會經過 server；完成與上一頁送對應訊息；放棄會通知 server 並關閉", async () => {
    const { onClose } = mount(login);
    await push(state({ mode: "login" }));
    await push(frame);
    expect(screen.getByText(/帳號密碼會經過 server 的記憶體/)).toBeInTheDocument();
    expect(screen.getByTestId("live-status")).toHaveTextContent("請在畫面上登入");
    expect(screen.queryByRole("button", { name: "接手" }), "登入不需要接手").toBeNull();
    const img = await sizeFrame();
    fireEvent.pointerDown(img, { clientX: 300, clientY: 150, button: 0 });
    expect(live.sent).toEqual([{ t: "mouse", type: "down", x: 0.5, y: 0.5, button: "left" }]);
    live.sent = [];
    fireEvent.click(screen.getByRole("button", { name: "上一頁" }));
    fireEvent.click(screen.getByRole("button", { name: "完成並保存 session" }));
    expect(live.sent).toEqual([{ t: "back" }, { t: "done" }]);
    fireEvent.click(screen.getByRole("button", { name: "放棄登入" }));
    expect(cancelRemoteLogin).toHaveBeenCalledWith("s1");
    expect(onClose).toHaveBeenCalled();
  });

  it("登入完成（server 回 session 已保存）通知呼叫端更新", async () => {
    const onLoginDone = vi.fn();
    mount(login, { onLoginDone });
    await push(state({ mode: "login" }));
    await push({ t: "closed", why: "登入完成，session 已保存" });
    expect(onLoginDone).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId("live-status")).toHaveTextContent("session 已保存");
  });
});
