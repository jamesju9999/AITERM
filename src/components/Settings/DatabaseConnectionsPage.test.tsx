import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi } from "vitest";
import { DatabaseConnectionsPage } from "./DatabaseConnectionsPage";
import { LocaleProvider } from "../../contexts/LocaleContext";

beforeEach(() => {
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
  dbRemoveConnection: vi.fn().mockResolvedValue(undefined),
  dbTestConnection: vi.fn().mockResolvedValue(undefined),
  DB_TYPE_LABELS: { postgresql: "PostgreSQL", mysql: "MySQL", sqlite: "SQLite", mssql: "MSSQL", db2: "DB2" },
  DB_DEFAULT_PORTS: { postgresql: 5432, mysql: 3306, sqlite: 0, mssql: 1433, db2: 50000 },
}));

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
