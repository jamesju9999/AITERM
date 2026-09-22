import React, { useEffect } from "react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, useLocation, useNavigate } from "react-router-dom";

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
    onCloseRequested: () => Promise.resolve(() => {}),
    destroy: () => Promise.resolve(),
  }),
}));
vi.mock("@tauri-apps/plugin-notification", () => ({ sendNotification: vi.fn() }));

// TerminalView 換成探針：這個檔案要驗的是「請求怎麼變成分頁」，不是 xterm。
vi.mock("./TerminalView", () => ({
  TerminalView: (p: { initialCwd?: string; initialCommand?: string; isActive?: boolean }) => (
    <div
      data-testid="tv"
      data-cwd={p.initialCwd ?? ""}
      data-cmd={p.initialCommand ?? ""}
      data-active={String(p.isActive)}
    />
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
import { saveSessionTabs } from "../lib/sessionTabs";

function mountApp() {
  return render(
    <MemoryRouter>
      <LocaleProvider>
        <TerminalApp />
      </LocaleProvider>
    </MemoryRouter>,
  );
}

// 每次路由變化（含首次渲染）記一筆，用來斷言「沒有多餘的導覽」。
const locationLog: string[] = [];
function LocationProbe() {
  const loc = useLocation();
  useEffect(() => {
    locationLog.push(loc.pathname);
  }, [loc]);
  const navigate = useNavigate();
  return (
    <div>
      <div data-testid="loc">{loc.pathname}</div>
      {/* 「上一頁」：用來數歷史堆疊到底推了幾筆。 */}
      <button data-testid="back" onClick={() => navigate(-1)} />
    </div>
  );
}

// App.tsx 在 pathname !== "/"（設定、引導）時把 TerminalApp 包在
// visibility:hidden + pointer-events:none 裡，所以要模擬「使用者正停在設定頁」。
function mountAppAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <LocationProbe />
      <LocaleProvider>
        <TerminalApp />
      </LocaleProvider>
    </MemoryRouter>,
  );
}

// StrictMode 在開發模式會把 effect 跑「掛載 → cleanup → 再掛載」。第一個實例被
// cleanup 之後，它那次 take 才回來，取走的請求就只存在那個回傳值裡。
function mountAppStrict() {
  return render(
    <React.StrictMode>
      <MemoryRouter>
        <LocaleProvider>
          <TerminalApp />
        </LocaleProvider>
      </MemoryRouter>
    </React.StrictMode>,
  );
}

// 腳本對話框出現後 600ms 內不接受「執行」（LaunchScriptConfirm 的防連點保護）。
// 這裡不用真的 sleep：凍結 Date／performance，再手動撥時間。這樣「雙擊的第二下落在
// 保護期內」這個前提不會被 CI 卡頓（GC、機器忙）弄破，測試也不必空等。
// 刻意不假造 setTimeout：waitFor／findBy／userEvent 內部要用真的計時器，否則會卡死。
const RUN_SHIELD_MS = 600;
const freezeClock = () => vi.useFakeTimers({ toFake: ["Date", "performance"] });

// 推過防連點保護期。兩件事都不能省，各自擋掉一種「執行被靜靜吞掉」的失敗：
//
// 1. **先把還沒跑的 effect flush 掉**。保護期的基準點是 LaunchScriptConfirm 在
//    mount effect 裡抓的 `performance.now()`，而且基準點還沒抓到時一律當成還在
//    保護期內（fail closed）。基準點若是在下面那行推進**之後**才抓的，抓到的就是
//    推進後的時間，保護期等於從頭開始，接著那一下「執行」會被吞掉——而且沒有任何
//    錯誤訊息，只會變成「分頁沒出現」。
// 2. **多推 50ms**。原本剛好推 RUN_SHIELD_MS，而元件的判斷式是
//    `performance.now() - shownAt < RUN_SHIELD_MS`，等於容錯是 0 毫秒。實測把推進
//    量改成 RUN_SHIELD_MS - 1，失敗訊息跟 CI 上那次偶發失敗一模一樣
//    （`expected [] to have a length of 1`）——那次就是這樣來的。
const pastRunShield = async () => {
  await act(async () => {});
  vi.advanceTimersByTime(RUN_SHIELD_MS + 50);
};

const tabsWith = (attr: "data-cwd" | "data-cmd", value: string) =>
  screen.queryAllByTestId("tv").filter((el) => el.getAttribute(attr) === value);

afterEach(() => {
  vi.useRealTimers();
});

beforeEach(() => {
  pending.length = 0;
  listeners.clear();
  localStorage.clear();
  takeImpl = defaultTake;
  takeCalls = 0;
  locationLog.length = 0;
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

  it("the launched tab becomes the active one and dismisses the home screen", async () => {
    // 冷啟動時首頁是前景、還原的分頁都在背景。從 Finder 開資料夾的使用者要看到的是
    // 剛開的那個分頁，不是首頁、也不是上次的分頁。isActive 包含 !homeActive，所以
    // 「只有新分頁 active」同時證明首頁被關掉了。
    saveSessionTabs([
      { id: "r1", title: "Terminal", type: "terminal", cwd: "/restored/a" },
      { id: "r2", title: "Terminal", type: "terminal", cwd: "/restored/b" },
    ]);
    pending.push({ cwd: "/tmp/new", script: null, command: null });
    mountApp();
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/new")).toHaveLength(1));
    expect(tabsWith("data-cwd", "/restored/a")).toHaveLength(1);
    expect(tabsWith("data-cwd", "/restored/b")).toHaveLength(1);
    const active = (el: HTMLElement) => el.getAttribute("data-active");
    expect(active(tabsWith("data-cwd", "/tmp/new")[0])).toBe("true");
    const others = screen.queryAllByTestId("tv").filter((el) => el.getAttribute("data-cwd") !== "/tmp/new");
    expect(others).toHaveLength(2);
    for (const el of others) expect(active(el)).toBe("false");
  });

  it("a -e request opens a tab carrying the quoted command line", async () => {
    pending.push({ cwd: "/tmp/proj", script: null, command: ["ls", "-la", "/tmp/a b"] });
    mountApp();
    await waitFor(() => expect(tabsWith("data-cmd", "ls -la '/tmp/a b'")).toHaveLength(1));
  });

  it("a script asks first: nothing runs until the user confirms", async () => {
    freezeClock();
    pending.push({ cwd: "/tmp/proj", script: "/tmp/proj/go.command", command: null });
    mountApp();
    await screen.findByRole("dialog");
    expect(screen.getByRole("dialog")).toHaveTextContent("/tmp/proj/go.command");
    expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(0);
    await pastRunShield();
    await userEvent.click(screen.getByTestId("launch-script-run"));
    // 先斷言對話框關掉，再斷言分頁。這一行把兩種完全不同的失敗分開：對話框還開著
    // ＝那一下「執行」被防連點保護吞了（保護期的基準點沒對齊，見 pastRunShield 的
    // 註解）；對話框關了卻沒有分頁＝建分頁那條路壞了。反過來寫的話，兩種都只會變成
    // 「expected [] to have a length of 1」，看不出是哪一種——CI 上那次偶發失敗就是
    // 這樣，得回頭重建現場才知道發生什麼事。
    expect(screen.queryByRole("dialog")).toBeNull();
    await waitFor(() => expect(tabsWith("data-cmd", "/tmp/proj/go.command")).toHaveLength(1));
    expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(1);
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

  it("a double-click on Run does not approve the NEXT queued script the user never read", async () => {
    pending.push(
      { cwd: "/a", script: "/a/one.command", command: null },
      { cwd: "/b", script: "/b/two.command", command: null },
    );
    freezeClock();
    mountApp();
    await screen.findByRole("dialog");
    await pastRunShield();
    await userEvent.click(screen.getByTestId("launch-script-run"));
    // 第一個腳本已核准；第二個對話框在同一個位置重新掛載。雙擊的第二下立刻落在它的「執行」上。
    expect(screen.getByRole("dialog")).toHaveTextContent("/b/two.command");
    await userEvent.click(screen.getByTestId("launch-script-run"));
    expect(tabsWith("data-cmd", "/a/one.command")).toHaveLength(1);
    expect(tabsWith("data-cmd", "/b/two.command")).toHaveLength(0);
    expect(screen.getByRole("dialog")).toHaveTextContent("/b/two.command");
    // 使用者真的讀過之後再點，就照常執行。
    await pastRunShield();
    await userEvent.click(screen.getByTestId("launch-script-run"));
    await waitFor(() => expect(tabsWith("data-cmd", "/b/two.command")).toHaveLength(1));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("the next queued script starts on the SAFE button even after the user clicked Run on the previous one", async () => {
    pending.push(
      { cwd: "/a", script: "/a/one.command", command: null },
      { cwd: "/b", script: "/b/two.command", command: null },
    );
    freezeClock();
    mountApp();
    await screen.findByRole("dialog");
    // 滑鼠點「執行」會把焦點留在「執行」上；如果第二個對話框重用同一個元件實例，
    // autoFocus 不會重新觸發，焦點就還在「執行」上。
    await pastRunShield();
    await userEvent.click(screen.getByTestId("launch-script-run"));
    await waitFor(() => expect(screen.getByRole("dialog")).toHaveTextContent("/b/two.command"));
    expect(screen.getByTestId("launch-script-skip")).toHaveFocus();
  });
});

describe("TerminalApp launch requests under StrictMode", () => {
  it("a request queued before mount opens exactly one tab", async () => {
    pending.push({ cwd: "/tmp/proj", script: null, command: null });
    mountAppStrict();
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 50));
    expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(1);
  });

  it("a request taken by the first (cleaned-up) effect instance and resolved late is not lost", async () => {
    // 第一次 take（屬於第一個、隨後被 cleanup 的 effect 實例）要等我們放行才 resolve；
    // 之後的 take（第二個實例）立刻回空。請求只存在第一次 take 的回傳值裡。
    let releaseFirst!: (v: LaunchReq[]) => void;
    let first = true;
    takeImpl = () => {
      if (first) {
        first = false;
        return new Promise<LaunchReq[]>((r) => { releaseFirst = r; });
      }
      return Promise.resolve([]);
    };
    mountAppStrict();
    await waitFor(() => expect(takeCalls).toBeGreaterThanOrEqual(2));
    await new Promise((r) => setTimeout(r, 30));
    expect(tabsWith("data-cwd", "/late/one")).toHaveLength(0);

    releaseFirst([{ cwd: "/late/one", script: null, command: null }]);
    await waitFor(() => expect(tabsWith("data-cwd", "/late/one")).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 30));
    expect(tabsWith("data-cwd", "/late/one")).toHaveLength(1);
  });

  it("a script request shows one dialog and Skip opens exactly one tab", async () => {
    pending.push({ cwd: "/s/proj", script: "/s/proj/go.command", command: null });
    mountAppStrict();
    await screen.findByRole("dialog");
    await new Promise((r) => setTimeout(r, 30));
    expect(screen.queryAllByRole("dialog")).toHaveLength(1);

    await userEvent.click(screen.getByTestId("launch-script-skip"));
    await waitFor(() => expect(tabsWith("data-cwd", "/s/proj")).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 50));
    expect(tabsWith("data-cwd", "/s/proj")).toHaveLength(1);
    expect(tabsWith("data-cmd", "/s/proj/go.command")).toHaveLength(0);
  });
});

describe("TerminalApp launch requests while the terminal is hidden behind another route", () => {
  it("a folder request brings the terminal view back", async () => {
    pending.push({ cwd: "/tmp/proj", script: null, command: null });
    mountAppAt("/settings");
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(1));
    await waitFor(() => expect(screen.getByTestId("loc")).toHaveTextContent(/^\/$/));
  });

  it("a script request also brings it back, so the confirmation dialog is actually visible", async () => {
    pending.push({ cwd: "/tmp/proj", script: "/tmp/proj/go.command", command: null });
    mountAppAt("/onboarding");
    await screen.findByRole("dialog");
    await waitFor(() => expect(screen.getByTestId("loc")).toHaveTextContent(/^\/$/));
  });

  it("does not navigate at all when the terminal is already showing", async () => {
    pending.push({ cwd: "/tmp/proj", script: null, command: null });
    mountAppAt("/");
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/proj")).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 30));
    expect(screen.getByTestId("loc")).toHaveTextContent(/^\/$/);
    // 只有首次渲染那一筆；沒有多推任何歷史。
    expect(locationLog).toEqual(["/"]);
  });

  it("two requests arriving together push only one history entry", async () => {
    pending.push(
      { cwd: "/tmp/a", script: null, command: null },
      { cwd: "/tmp/b", script: null, command: null },
    );
    mountAppAt("/settings");
    await waitFor(() => expect(tabsWith("data-cwd", "/tmp/b")).toHaveLength(1));
    await new Promise((r) => setTimeout(r, 30));
    expect(locationLog).toEqual(["/settings", "/"]);
    // 只推了一筆：往回一步就是原本的設定頁。多推的話會停在 "/"。
    await userEvent.click(screen.getByTestId("back"));
    await waitFor(() => expect(screen.getByTestId("loc")).toHaveTextContent("/settings"));
  });
});
