import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

import { McpServersPage } from "./McpServersPage";

vi.mock("../../ipc/mcp", () => ({
  listMcpServers: vi.fn(),
  removeMcpServer: vi.fn(),
  importClaudeDesktopMcp: vi.fn(),
  addMcpServer: vi.fn(),
  setMcpEnabled: vi.fn(),
  getMcpTools: vi.fn().mockResolvedValue([]),
}));
vi.mock("../../ipc/config", () => ({ getConfig: vi.fn() }));

import { listMcpServers } from "../../ipc/mcp";
import { getConfig } from "../../ipc/config";
import type { AppConfig } from "../../ipc/config";

describe("McpServersPage — 載入失敗要看得見", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  /**
   * 這條守的是一個實際發生過、而且很嚇人的失效模式。
   *
   * `reload()` 原本沒有錯誤處理：`listMcpServers()` 一旦拋出或永遠不回，
   * `setServers` 與 `setMcpEnabledState` 都不會執行——清單留在初始的空陣列、
   * 勾選框留在初始的 `true`。畫面呈現的就是「MCP 開著，但一個 server 都沒有」，
   * 跟「使用者從來沒設定過」長得一模一樣。使用者的實際反應是「我的設定突然
   * 不見了」，但設定檔其實完好無損。
   *
   * 所以錯誤訊息除了說失敗，還要明講「設定檔沒有被更動」。
   */
  it("讀取失敗時顯示錯誤，而不是偽裝成「尚無 MCP Server」", async () => {
    vi.mocked(listMcpServers).mockRejectedValue(new Error("lock timeout"));
    vi.mocked(getConfig).mockResolvedValue({ mcp_enabled: true } as AppConfig);

    render(<McpServersPage />);

    await waitFor(() => {
      expect(screen.getByText(/讀取 MCP Server 清單失敗/)).toBeInTheDocument();
    });
    expect(
      screen.queryByText(/尚無 MCP Server/),
      "失敗時不可顯示空狀態——那會讓使用者以為設定被刪了",
    ).not.toBeInTheDocument();
    // 錯誤訊息要安撫「資料還在」，因為使用者的第一反應是以為設定掉了。
    expect(screen.getByText(/設定檔沒有被更動/)).toBeInTheDocument();
  });

  it("成功但真的沒有 server 時，才顯示空狀態", async () => {
    vi.mocked(listMcpServers).mockResolvedValue([]);
    vi.mocked(getConfig).mockResolvedValue({ mcp_enabled: true } as AppConfig);

    render(<McpServersPage />);

    await waitFor(() => {
      expect(screen.getByText(/尚無 MCP Server/)).toBeInTheDocument();
    });
    expect(screen.queryByText(/讀取 MCP Server 清單失敗/)).not.toBeInTheDocument();
  });
});
