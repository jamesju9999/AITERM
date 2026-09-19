import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

type LaunchReq = { cwd: string | null; script: string | null; command: string[] | null };
const pending: LaunchReq[] = [];
const listeners = new Map<string, () => void>();
// 預設的 take 實作：一次取走整個佇列（後端是原子的）。個別測試可以換掉它來
// 控制「哪一次呼叫在什麼時候 resolve」。
const defaultTake = () => Promise.resolve(pending.splice(0));
let takeImpl: () => Promise<LaunchReq[]> = defaultTake;
let takeCalls = 0;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn((cmd: string) => {
    if (cmd === "take_launch_requests") {
      takeCalls += 1;
      return takeImpl();
    }
    return new Promise(() => {});
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, cb: () => void) => {
    listeners.set(name, cb);
    return Promise.resolve(() => listeners.delete(name));
  }),
}));
vi.mock("@tauri-apps/api/path", () => ({ homeDir: vi.fn(() => Promise.resolve("/home/test")) }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isFocused: () => Promise.resolve(true),
    onFocusChanged: () => Promise.resolve(() => {}),
    isMaximized: () => Promise.resolve(false),
    onResized: () => Promise.resolve(() => {}),
    maximize: () => Promise.resolve(),
    unmaximize: () => Promise.resolve(),
    minimize: () => Promise.resolve(),
    close: () => Promise.resolve(),
    startDragging: () => Promise.resolve(),
  }),
}));
vi.mock("@tauri-apps/plugin-notification", () => ({ sendNotification: vi.fn() }));

// TerminalView 換成探針：這個檔案要驗的是「請求怎麼變成分頁」，不是 xterm。
vi.mock("./TerminalView", () => ({
  TerminalView: (p: { initialCwd?: string; initialCommand?: string }) => (
    <div data-testid="tv" data-cwd={p.initialCwd ?? ""} data-cmd={p.initialCommand ?? ""} />
  ),
}));

Element.prototype.scrollTo = Element.prototype.scrollTo || (() => {});
if (!window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}
class FakeResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
(globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = FakeResizeObserver;

import { TerminalApp } from "./TerminalApp";
import { LocaleProvider } from "../contexts/LocaleContext";

function mountApp() {
  return render(
    <MemoryRouter>
      <LocaleProvider>
        <TerminalApp />
      </LocaleProvider>
    </MemoryRouter>,
  );
}

const tabsWith = (attr: "data-cwd" | "data-cmd", value: string) =>
  screen.queryAllByTestId("tv").filter((el) => el.getAttribute(attr) === value);

beforeEach(() => {
  pending.length = 0;
  listeners.clear();
  localStorage.clear();
  takeImpl = defaultTake;
  takeCalls = 0;
});

describe("TerminalApp launch requests", () => {
  it("sanity: with no request, no tab has a starting directory", async () => {
    mountApp();
    await waitFor(() => expect(screen.queryAllByTestId("tv").length).toBeGreaterThan(0));
    expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(0);
  });

  it("a request queued BEFORE the app mounted still opens a tab at that directory", async () => {
    pending.push({ cwd: "/tmp/proj", script: null, command: null });
    mountApp();
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(1));
  });

  it("a request queued AFTER mount opens a tab when the pending event fires", async () => {
    mountApp();
    await waitFor(() => expect(listeners.has("launch-request-pending")).toBe(true));
    pending.push({ cwd: "/tmp/later", script: null, command: null });
    listeners.get("launch-request-pending")!();
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/later")).toHaveLength(1));
  });

  it("firing the event twice does not open the same request twice", async () => {
    mountApp();
    await waitFor(() => expect(listeners.has("launch-request-pending")).toBe(true));
    pending.push({ cwd: "/tmp/once", script: null, command: null });
    listeners.get("launch-request-pending")!();
    listeners.get("launch-request-pending")!();
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/once")).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 30));
    expect(tabsWith("data-cwd", "/tmp/once")).toHaveLength(1);
  });

  it("overlapping drains are processed in the order they were issued, not the order they resolve", async () => {
    mountApp();
    await waitFor(() => expect(listeners.has("launch-request-pending")).toBe(true));
    // 等啟動時那一次排空完全結束，之後才換掉 take 實作。
    await waitFor(() => expect(takeCalls).toBeGreaterThanOrEqual(1));
    await new Promise((r) => setTimeout(r, 0));

    // 第一次 take 拿到 [A] 但很晚才 resolve；第二次 take 拿到 [B] 且立刻 resolve。
    // 沒有串行化的話，B 會先被處理（先開分頁）。
    let releaseA!: (v: LaunchReq[]) => void;
    const responses: Array<Promise<LaunchReq[]>> = [
      new Promise<LaunchReq[]>((r) => { releaseA = r; }),
      Promise.resolve([{ cwd: "/order/B", script: null, command: null }]),
    ];
    takeImpl = () => responses.shift() ?? Promise.resolve([]);

    listeners.get("launch-request-pending")!();
    listeners.get("launch-request-pending")!();
    // 讓所有「不需要等 A」的工作跑完：沒串行化時，B 此刻已經開了分頁。
    await new Promise((r) => setTimeout(r, 30));
    releaseA([{ cwd: "/order/A", script: null, command: null }]);

    await waitFor(() => expect(tabsWith("data-cwd", "/order/B")).toHaveLength(1));
    const order = screen
      .queryAllByTestId("tv")
      .map((el) => el.getAttribute("data-cwd"))
      .filter((c) => c?.startsWith("/order/"));
    expect(order).toEqual(["/order/A", "/order/B"]);
  });

  it("a -e request opens a tab carrying the quoted command line", async () => {
    pending.push({ cwd: "/tmp/proj", script: null, command: ["ls", "-la", "/tmp/a b"] });
    mountApp();
    await waitFor(() => expect(tabsWith("data-cmd", "ls -la '/tmp/a b'")).toHaveLength(1));
  });

  it("a script asks first: nothing runs until the user confirms", async () => {
    pending.push({ cwd: "/tmp/proj", script: "/tmp/proj/go.command", command: null });
    mountApp();
    await screen.findByRole("dialog");
    expect(screen.getByRole("dialog")).toHaveTextContent("/tmp/proj/go.command");
    expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(0);
    await userEvent.click(screen.getByTestId("launch-script-run"));
    await waitFor(() => expect(tabsWith("data-cmd", "/tmp/proj/go.command")).toHaveLength(1));
    expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(1);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("skipping a script still opens the directory but runs nothing", async () => {
    pending.push({ cwd: "/tmp/proj", script: "/tmp/proj/go.command", command: null });
    mountApp();
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByTestId("launch-script-skip"));
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(1));
    expect(tabsWith("data-cmd", "/tmp/proj/go.command")).toHaveLength(0);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("two scripts are confirmed one at a time, in order", async () => {
    pending.push(
      { cwd: "/a", script: "/a/one.command", command: null },
      { cwd: "/b", script: "/b/two.command", command: null },
    );
    mountApp();
    const first = await screen.findByRole("dialog");
    expect(first).toHaveTextContent("/a/one.command");
    await userEvent.click(screen.getByTestId("launch-script-skip"));
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("/b/two.command"));
  });

  it("the next queued script starts on the SAFE button even after the user clicked Run on the previous one", async () => {
    pending.push(
      { cwd: "/a", script: "/a/one.command", command: null },
      { cwd: "/b", script: "/b/two.command", command: null },
    );
    mountApp();
    await screen.findByRole("dialog");
    // 滑鼠點「執行」會把焦點留在「執行」上；如果第二個對話框重用同一個元件實例，
    // autoFocus 不會重新觸發，焦點就還在「執行」上。
    await userEvent.click(screen.getByTestId("launch-script-run"));
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("/b/two.command"));
    expect(screen.getByTestId("launch-script-skip")).toHaveFocus();
  });
});
