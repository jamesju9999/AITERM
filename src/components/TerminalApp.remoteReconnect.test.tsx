import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";

// 跟 RemoteTerminalView/index.test.tsx 的 disconnectMock 同一套手法：這個
// 檔案的 RemoteTerminalView 是 stub（見下面），不會呼叫真正的
// shareViewerDisconnect；這裡測的是 TerminalApp.tsx 自己在「重新連線的
// 分頁被 Ctrl+W 關掉」時，直接呼叫這支 IPC 斷開孤兒連線的那條防線。
const disconnectMock = vi.fn().mockResolvedValue(undefined);
vi.mock("../ipc/shareViewer", () => ({
  shareViewerDisconnect: (...a: unknown[]) => disconnectMock(...a),
}));

// 跟既有的 TerminalApp.routeHintCloseGuard.test.tsx 同一套 mount probe
// 結論：這幾個底層 Tauri 入口點涵蓋 TerminalApp 掛載所需的一切。
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(() => new Promise(() => {})) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
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
vi.mock("@tauri-apps/plugin-notification", () => ({
  sendNotification: vi.fn(),
}));

// 這個測試檔案只關心 TerminalApp 怎麼管理 tabs 陣列（開新分頁 vs. 更新
// 既有分頁），不關心 RemoteTerminalView 或 ConnectDialog 各自內部的真實
// 行為（那些各自有自己的測試檔案）——兩個都 stub 成暴露必要 props 的
// 簡單元件，讓這個測試檔案可以直接觸發 TerminalApp 的狀態機邏輯，不用
// 處理一整套 share-viewer IPC/xterm mock。
type RemoteTerminalViewProps = {
  tabId: string;
  connId: string;
  hostLabel?: string;
  onConnectClick: () => void;
};
vi.mock("./RemoteTerminalView", () => ({
  RemoteTerminalView: ({ tabId, connId, hostLabel, onConnectClick }: RemoteTerminalViewProps) => (
    <div>
      <div data-testid={`remote-conn-${tabId}`}>{connId}:{hostLabel}</div>
      <button onClick={onConnectClick}>connect-button-{tabId}</button>
    </div>
  ),
}));

type ConnectDialogProps = {
  onConnected: (connId: string, sas: string, hostLabel: string) => void;
  onCancel: () => void;
};
vi.mock("./ConnectDialog", () => ({
  ConnectDialog: ({ onConnected, onCancel }: ConnectDialogProps) => (
    <div>
      <button onClick={() => onConnected("new-conn-id", "1234", "10.0.0.9:9999")}>
        fire-connected
      </button>
      <button onClick={onCancel}>fire-cancel</button>
    </div>
  ),
}));

// 同一個理由：這個測試檔案只關心「選了 remote-terminal 類型之後
// TerminalApp 怎麼分流」，不重新測 NewTabPicker 自己的清單/搜尋邏輯
// （那是它自己測試檔案的職責）。
type NewTabPickerProps = { onSelect: (type: string) => void };
vi.mock("./NewTabPicker", () => ({
  NewTabPicker: ({ onSelect }: NewTabPickerProps) => (
    <button onClick={() => onSelect("remote-terminal")}>pick-remote-terminal</button>
  ),
}));

import { TerminalApp } from "./TerminalApp";
import { LocaleProvider } from "../contexts/LocaleContext";
import { SESSION_TABS_KEY } from "../lib/sessionTabs";

beforeEach(() => {
  disconnectMock.mockClear();
  localStorage.clear();
  // 起始狀態：一個既有的遠端終端機分頁。還原機制（sessionTabs.ts 的
  // SavedTab）本來就不存 remoteConnId/remoteSas/remoteHostLabel（那些是
  // 執行期才有意義的欄位），所以還原出來的分頁這三個欄位都是空的——這對
  // 這裡的測試沒有影響：要驗證的是「點連線鈕、完成連線流程後，這個分頁
  // 的欄位被更新成新值」，起始值是不是空字串不影響這個行為本身。
  localStorage.setItem(
    SESSION_TABS_KEY,
    JSON.stringify([{ title: "遠端終端機：舊主機", type: "remote-terminal" }]),
  );
});

function renderApp() {
  return render(
    <LocaleProvider>
      <MemoryRouter>
        <TerminalApp />
      </MemoryRouter>
    </LocaleProvider>,
  );
}

describe("TerminalApp: 遠端終端機工具列的「連線」按鈕就地重新連線", () => {
  it("點既有分頁的連線按鈕、完成連線流程後，更新同一個分頁而不是開新分頁", async () => {
    renderApp();

    // 起始畫面是首頁（homeActive 預設 true），分頁本身用
    // visibility:hidden + pointer-events:none 蓋著（見 TerminalApp.tsx
    // 的 HIDDEN LAYOUT TRICK 註解）——要先切到這個分頁，連線按鈕才點
    // 得到。這跟既有的 TerminalApp.routeHintCloseGuard.test.tsx 是同一個
    // 前提，那邊用 HomeView 的 stub 按鈕切換，這裡因為沒有 mock
    // HomeView，改直接點側邊欄的分頁項目。
    await userEvent.click(await screen.findByTitle(/^Switch to Tab \(Ctrl\+1\)/));

    const connectButtons = await screen.findAllByText(/^connect-button-/);
    expect(connectButtons).toHaveLength(1);
    await userEvent.click(connectButtons[0]);

    const fireConnected = await screen.findByText("fire-connected");
    await userEvent.click(fireConnected);

    // 還是只有一個遠端終端機分頁的畫面——沒有多開一個。
    await screen.findByTestId(/remote-conn-/);
    const remoteViews = screen.getAllByTestId(/remote-conn-/);
    expect(remoteViews).toHaveLength(1);
    expect(remoteViews[0].textContent).toBe("new-conn-id:10.0.0.9:9999");
  });

  it("點連線按鈕後按取消，不影響後續從 ADD TAB 開新分頁的既有流程", async () => {
    renderApp();

    // 同上一個測試：先切到這個分頁，連線按鈕才點得到。
    await userEvent.click(await screen.findByTitle(/^Switch to Tab \(Ctrl\+1\)/));

    const connectButtons = await screen.findAllByText(/^connect-button-/);
    await userEvent.click(connectButtons[0]);

    const fireCancel = await screen.findByText("fire-cancel");
    await userEvent.click(fireCancel);

    // 對話框關閉、沒有任何分頁被更新——原本的分頁欄位維持原樣（起始值
    // 是空字串，因為 sessionTabs 還原本來就不存這幾個欄位）。
    const remoteViewsAfterCancel = screen.getAllByTestId(/remote-conn-/);
    expect(remoteViewsAfterCancel).toHaveLength(1);
    expect(remoteViewsAfterCancel[0].textContent).toBe(":");
  });

  it("沒有先點過任何分頁的連線鈕、直接從 ADD TAB 開新分頁，仍然正常開新分頁", async () => {
    // 確保這次改動沒有破壞既有的「開新分頁」路徑——這裡完全不碰任何
    // 既有分頁的連線鈕，直接走 ADD TAB → 選 remote-terminal 類型 →
    // ConnectDialog 這條路，reconnectTabId 應該從頭到尾都是 null。
    renderApp();

    const beforeCount = screen.getAllByTestId(/remote-conn-/).length;
    expect(beforeCount).toBe(1);

    await userEvent.click(await screen.findByTitle("New Tab (Ctrl+T)"));
    await userEvent.click(await screen.findByText("pick-remote-terminal"));

    const fireConnected = await screen.findByText("fire-connected");
    await userEvent.click(fireConnected);

    // 開了一個新分頁——現在應該有兩個遠端終端機分頁的畫面，原本那個
    // 分頁的內容維持原樣（起始的空字串），新分頁帶著剛才連線的內容。
    const remoteViewsAfterAdd = await screen.findAllByTestId(/remote-conn-/);
    expect(remoteViewsAfterAdd).toHaveLength(2);
    const texts = remoteViewsAfterAdd.map((el) => el.textContent).sort();
    expect(texts).toEqual([":", "new-conn-id:10.0.0.9:9999"]);
  });

  it("Ctrl+W 關掉正在重新連線的分頁後，完成對話框流程不會弄出幽靈分頁，會斷開孤兒連線", async () => {
    // 這裡刻意用兩個既有的遠端終端機分頁，不是這個檔案 beforeEach 預設的
    // 一個：只有一個分頁時，Ctrl+W 關掉它會走 handleCloseTab「關掉最後一個
    // 分頁，自動補一個全新 Terminal」那條路，會掛載真正的 TerminalView
    // （這個檔案沒有為它準備 xterm/useTerminalBlocks 之類的完整 mock 組）。
    // 兩個分頁時，Ctrl+W 走的是「切到鄰居分頁」那條路，不會碰到 TerminalView。
    localStorage.setItem(
      SESSION_TABS_KEY,
      JSON.stringify([
        { title: "遠端終端機：舊主機", type: "remote-terminal" },
        { title: "遠端終端機：另一台", type: "remote-terminal" },
      ]),
    );
    renderApp();

    await userEvent.click(await screen.findByTitle(/^Switch to Tab \(Ctrl\+1\)/));
    const connectButtons = await screen.findAllByText(/^connect-button-/);
    await userEvent.click(connectButtons[0]);
    const fireConnected = await screen.findByText("fire-connected");

    // 刻意用 fireEvent（同步）而不是 userEvent（內部會 await），且兩個
    // 呼叫之間不插入任何 await：這樣 handleCloseTab 內 `await
    // runCloseGuard(...)` 之後那段清空 reconnectTabId／移除分頁的程式碼
    // 還沒機會執行，onConnected 觸發時 reconnectTabId 依然指著這個分頁
    // ——這就是 Ctrl+W 真正「跟對話框完成流程搶時間」的那個時間點，不是
    // 兩個先後分開、各自流程都跑完的操作。經驗證：這個順序穩定重現得到
    // 「連線完成當下，目標分頁其實已經不在了」這個情境，不是碰運氣的
    // flaky 寫法（同一段同步程式碼的執行順序在 JS 裡是決定性的）。
    fireEvent.keyDown(window, { key: "w", ctrlKey: true });
    fireEvent.click(fireConnected);

    // 讓 handleCloseTab 剩下的非同步尾段（清空 reconnectTabId、真正移除
    // 分頁）跑完。
    await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 0));

    // 對話框關閉，沒有拋錯、沒有卡住。
    expect(screen.queryByText("fire-connected")).not.toBeInTheDocument();

    // 沒有幽靈分頁：畫面上只剩下沒被動過的「另一台」那個分頁，被關掉的
    // 那個分頁、還有它原本要接手的新連線都不在畫面上。
    const remoteViews = screen.getAllByTestId(/remote-conn-/);
    expect(remoteViews).toHaveLength(1);
    expect(remoteViews[0].textContent).toBe(":");

    // 剛從 ConnectDialog 建立好、沒有任何分頁接手的連線，明確斷開了，
    // 不留給後端逾時才清。
    expect(disconnectMock).toHaveBeenCalledTimes(1);
    expect(disconnectMock).toHaveBeenCalledWith("new-conn-id");
  });
});
