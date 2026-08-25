import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";

// 設定頁只是被掛起來檢查外框，內容分頁一律不渲染——它們各自有自己的測試，
// 而且會拉進一堆 IPC。
vi.mock("./GeneralPage", () => ({ GeneralPage: () => null }));
vi.mock("./ProvidersPage", () => ({ ProvidersPage: () => null }));
vi.mock("./DatabaseConnectionsPage", () => ({ DatabaseConnectionsPage: () => null }));
vi.mock("./VcsConnectionsPage", () => ({ VcsConnectionsPage: () => null }));
vi.mock("./McpServersPage", () => ({ McpServersPage: () => null }));
vi.mock("./ClaudeBridgePage", () => ({ ClaudeBridgePage: () => null }));
vi.mock("./McpToolServerPage", () => ({ McpToolServerPage: () => null }));
vi.mock("./QuotaPage", () => ({ QuotaPage: () => null }));
vi.mock("./AboutPage", () => ({ AboutPage: () => null }));

import { SettingsView } from "./SettingsView";
import { LocaleProvider } from "../../contexts/LocaleContext";

describe("設定頁的視窗拖曳區", () => {
  // 設定頁是蓋在 TerminalApp 上的覆蓋層，而 TerminalApp（連同帶
  // data-tauri-drag-region 的 TitleBar／TabBar）在這時候是 visibility: hidden，
  // 所以設定頁必須自己提供拖曳區，否則使用者在這一頁完全拖不動視窗，
  // 只能先切回終端機。
  function renderSettings() {
    const { container } = render(
      <MemoryRouter initialEntries={["/settings"]}>
        <LocaleProvider>
          <SettingsView />
        </LocaleProvider>
      </MemoryRouter>
    );
    return container;
  }

  it("側邊欄是 Tauri 拖曳區", () => {
    const sidebar = renderSettings().querySelector(".settings-sidebar");
    expect(sidebar).not.toBeNull();
    expect(sidebar).toHaveAttribute("data-tauri-drag-region");
  });

  // Tauri 只在事件目標「自己」是拖曳區時才開始拖，屬性不會從父層繼承下來。
  // 這兩個子元素蓋掉了側邊欄絕大部分的可見面積：spacer 是 flex:1，吃掉所有
  // 垂直空白（畫面上那一大片空的就是它）；標題則佔著最上緣、最靠近使用者
  // 習慣去抓的位置。少了它們，實際抓得到的只剩邊距和按鈕之間幾 px 的縫，
  // 使用者的感受就是「還是拖不動」。
  it("撐開空白的 spacer 也是拖曳區", () => {
    const spacer = renderSettings().querySelector(".sidebar-spacer");
    expect(spacer).not.toBeNull();
    expect(spacer).toHaveAttribute("data-tauri-drag-region");
  });

  it("最上方的標題也是拖曳區", () => {
    const title = renderSettings().querySelector(".settings-sidebar-title");
    expect(title).not.toBeNull();
    expect(title).toHaveAttribute("data-tauri-drag-region");
  });
});
