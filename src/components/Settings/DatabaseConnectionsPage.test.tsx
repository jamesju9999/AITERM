import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi } from "vitest";
import { DatabaseConnectionsPage } from "./DatabaseConnectionsPage";
import { LocaleProvider } from "../../contexts/LocaleContext";

beforeEach(() => {
  vi.clearAllMocks();
  Object.defineProperty(window, "localStorage", {
    value: { getItem: vi.fn(() => null), setItem: vi.fn(), removeItem: vi.fn(), clear: vi.fn(), key: vi.fn(), length: 0 },
    writable: true,
  });
});

function renderPage() {
  return render(<LocaleProvider><DatabaseConnectionsPage /></LocaleProvider>);
}

vi.mock("../../ipc/db", () => ({
  dbListConnections: vi.fn().mockResolvedValue([]),
  dbAddConnection: vi.fn().mockResolvedValue("new-id"),
  dbUpdateConnection: vi.fn().mockResolvedValue(undefined),
  dbRemoveConnection: vi.fn().mockResolvedValue(undefined),
  dbTestConnection: vi.fn().mockResolvedValue(undefined),
  dbCheckImportFile: vi.fn(),
  dbExportConnections: vi.fn(),
  dbPreviewImport: vi.fn(),
  dbImportConnections: vi.fn(),
  DB_TYPE_LABELS: { postgresql: "PostgreSQL", mysql: "MySQL", sqlite: "SQLite", mssql: "MSSQL", db2: "DB2" },
  DB_DEFAULT_PORTS: { postgresql: 5432, mysql: 3306, sqlite: 0, mssql: 1433, db2: 50000 },
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(), open: vi.fn() }));

import { open } from "@tauri-apps/plugin-dialog";
import {
  dbListConnections, dbCheckImportFile, dbPreviewImport, dbImportConnections,
} from "../../ipc/db";

const ONE_CONN = [{
  id: "a", name: "總行LBOTHODB", db_type: "db2" as const, host: "172.19.2.83", port: 25000,
  database: "LBOTHODB", username: "nuntio", default_schema: "NUNTIO", is_connected: true,
}];

describe("DatabaseConnectionsPage", () => {
  it("renders empty state and add button", async () => {
    renderPage();
    await waitFor(() => expect(screen.getByText("+ 新增連線")).toBeInTheDocument());
  });

  it("shows form when + button clicked", async () => {
    renderPage();
    await waitFor(() => fireEvent.click(screen.getByText("+ 新增連線")));
    expect(screen.getByText("新增連線")).toBeInTheDocument();
  });

  it("shows DB2 ODBC notice when DB2 selected", async () => {
    renderPage();
    await waitFor(() => fireEvent.click(screen.getByText("+ 新增連線")));
    const select = screen.getByRole("combobox");
    fireEvent.change(select, { target: { value: "db2" } });
    expect(screen.getByText(/IBM DB2 ODBC Driver/)).toBeInTheDocument();
  });
});

describe("DatabaseConnectionsPage transfer buttons", () => {
  it("disables export when there are no connections", async () => {
    vi.mocked(dbListConnections).mockResolvedValue([]);
    renderPage();
    await waitFor(() => expect(screen.getByRole("button", { name: "匯出" })).toBeDisabled());
    expect(screen.getByRole("button", { name: "匯入" })).toBeEnabled();
  });

  it("opens the export panel and hides the connection list", async () => {
    vi.mocked(dbListConnections).mockResolvedValue(ONE_CONN);
    renderPage();
    // 必須先等清單載入完。「匯出」鈕在 connections 還是空陣列時是 disabled，
    // 而 waitFor 的第一次檢查是同步跑的——getByRole 當下就找得到那顆
    // （存在，只是 disabled），callback 不 throw 就立刻 resolve，於是點擊
    // 會落在 disabled 按鈕上變成 no-op。要等的是「鈕變成可按」，不是「鈕存在」。
    await waitFor(() => expect(screen.getByRole("button", { name: "匯出" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "匯出" }));
    expect(screen.getByText("匯出資料庫連線")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "+ 新增連線" })).not.toBeInTheDocument();
  });

  // 這是真正的回歸風險：匯入完成後若沒有重新拉一次清單，畫面會停在舊資料。
  it("reloads the connection list after an import finishes", async () => {
    vi.mocked(dbListConnections).mockResolvedValue(ONE_CONN);
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    vi.mocked(dbPreviewImport).mockResolvedValue([{
      id: "z", name: "新連線", db_type: "mysql", host: "10.0.0.5", port: 3306,
      database: "app", username: "root", conflict: "new", existing_name: null,
    }]);
    vi.mocked(dbImportConnections).mockResolvedValue({ added: 1, overwritten: 0, failures: [] });

    renderPage();
    await waitFor(() => expect(dbListConnections).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "匯入" }));

    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: "pw" } });
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    await waitFor(() => expect(screen.getByRole("checkbox")).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "匯入" }));

    await waitFor(() => expect(dbListConnections).toHaveBeenCalledTimes(2));
    // 匯入面板刻意留在畫面上，讓使用者看得到結果與失敗清單
    expect(screen.getByText("新增 1 筆、覆蓋 0 筆")).toBeInTheDocument();
  });
});
