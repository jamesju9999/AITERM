import { describe, it, expect } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi } from "vitest";
import { DatabaseConnectionsPage } from "./DatabaseConnectionsPage";

vi.mock("../../ipc/db", () => ({
  dbListConnections: vi.fn().mockResolvedValue([]),
  dbAddConnection: vi.fn().mockResolvedValue("new-id"),
  dbRemoveConnection: vi.fn().mockResolvedValue(undefined),
  dbTestConnection: vi.fn().mockResolvedValue(undefined),
  DB_TYPE_LABELS: { postgresql: "PostgreSQL", mysql: "MySQL", sqlite: "SQLite", mssql: "MSSQL", db2: "DB2" },
  DB_DEFAULT_PORTS: { postgresql: 5432, mysql: 3306, sqlite: 0, mssql: 1433, db2: 50000 },
}));

describe("DatabaseConnectionsPage", () => {
  it("renders empty state and add button", async () => {
    render(<DatabaseConnectionsPage />);
    await waitFor(() => expect(screen.getByText("+ 新增連線")).toBeInTheDocument());
  });

  it("shows form when + button clicked", async () => {
    render(<DatabaseConnectionsPage />);
    await waitFor(() => fireEvent.click(screen.getByText("+ 新增連線")));
    expect(screen.getByPlaceholderText("我的資料庫")).toBeInTheDocument();
  });

  it("shows DB2 ODBC notice when DB2 selected", async () => {
    render(<DatabaseConnectionsPage />);
    await waitFor(() => fireEvent.click(screen.getByText("+ 新增連線")));
    const select = screen.getByRole("combobox");
    fireEvent.change(select, { target: { value: "db2" } });
    expect(screen.getByText(/IBM DB2 ODBC Driver/)).toBeInTheDocument();
  });
});
