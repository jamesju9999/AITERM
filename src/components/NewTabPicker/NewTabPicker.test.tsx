import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi } from "vitest";

vi.mock("../../ipc/bridge", () => ({ bridgeStatus: vi.fn() }));

import { NewTabPicker } from "./index";
import type { Tab } from "../TabBar";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { bridgeStatus } from "../../ipc/bridge";

beforeEach(() => {
  Object.defineProperty(window, "localStorage", {
    value: { getItem: vi.fn(() => null), setItem: vi.fn(), removeItem: vi.fn(), clear: vi.fn(), key: vi.fn(), length: 0 },
    writable: true,
  });
  vi.mocked(bridgeStatus).mockReset();
  // 大多數既有測試不在乎橋接選項，預設回傳「執行中」讓它可點——
  // 只有專門測停用狀態的案例才覆寫成 false。
  vi.mocked(bridgeStatus).mockResolvedValue({ running: true, port: 8317, token: "tok", error: null });
});

function renderPicker(onSelect = vi.fn(), onClose = vi.fn()) {
  return render(
    <LocaleProvider>
      <NewTabPicker onSelect={onSelect} onClose={onClose} />
    </LocaleProvider>
  );
}

describe("Tab type", () => {
  it("accepts terminal type", () => {
    const tab: Tab = { id: "1", title: "Terminal", type: "terminal" };
    expect(tab.type).toBe("terminal");
  });

  it("accepts database type with connection id", () => {
    const tab: Tab = { id: "2", title: "DB", type: "database", dbConnectionId: "conn-1" };
    expect(tab.type).toBe("database");
    expect(tab.dbConnectionId).toBe("conn-1");
  });
});

describe("NewTabPicker", () => {
  it("renders two options when open", () => {
    renderPicker();
    expect(screen.getByText("終端機")).toBeInTheDocument();
    expect(screen.getByText("資料庫")).toBeInTheDocument();
  });

  it("calls onSelect with terminal when 終端機 clicked", () => {
    const onSelect = vi.fn();
    renderPicker(onSelect);
    fireEvent.click(screen.getByText("終端機"));
    expect(onSelect).toHaveBeenCalledWith("terminal");
  });

  it("calls onSelect with database when 資料庫 clicked", () => {
    const onSelect = vi.fn();
    renderPicker(onSelect);
    fireEvent.click(screen.getByText("資料庫"));
    expect(onSelect).toHaveBeenCalledWith("database");
  });

  it("calls onClose when Escape pressed", () => {
    const onClose = vi.fn();
    renderPicker(vi.fn(), onClose);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});

describe("NewTabPicker — Claude Code tab option", () => {
  it("強制帶 claudeBridge:true，橋接執行中時可點", async () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    renderPicker(onSelect, onClose);

    const button = await screen.findByText("Claude Code");
    await waitFor(() => expect(button.closest("button")).toBeEnabled());
    fireEvent.click(button);

    expect(onSelect).toHaveBeenCalledWith("terminal", { claudeBridge: true });
    expect(onClose).toHaveBeenCalled();
  });

  it("橋接沒在跑時停用該選項，並在 title 提示先到設定頁啟用", async () => {
    vi.mocked(bridgeStatus).mockResolvedValue({ running: false, port: null, token: null, error: null });
    const onSelect = vi.fn();
    renderPicker(onSelect);

    const button = await screen.findByText("Claude Code");
    const buttonEl = button.closest("button")!;
    await waitFor(() => expect(buttonEl).toBeDisabled());
    expect(buttonEl.title).toBe("橋接 server 尚未啟動，請先到設定頁啟用");

    fireEvent.click(buttonEl);
    expect(onSelect).not.toHaveBeenCalled();
  });
});
