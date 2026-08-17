import React from "react";
import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { vi } from "vitest";
import { describe, it, expect } from "vitest";
import { DatabaseView } from "./index";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockResolvedValue([]),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

function renderWithRouter(ui: React.ReactElement) {
  return render(<MemoryRouter><LocaleProvider>{ui}</LocaleProvider></MemoryRouter>);
}

vi.mock("../../ipc/db", () => ({
  dbListConnections: vi.fn().mockResolvedValue([
    { id: "c1", name: "My PG", db_type: "postgresql", host: "localhost",
      port: 5432, database: "mydb", username: "postgres", is_connected: false },
  ]),
  dbConnect: vi.fn().mockResolvedValue(undefined),
  dbListSchemas: vi.fn().mockResolvedValue(["public"]),
  dbListTables: vi.fn().mockResolvedValue([]),
  DB_TYPE_LABELS: { postgresql: "PostgreSQL" },
}));

describe("DatabaseView", () => {
  it("shows connection selector when no connection is set", async () => {
    renderWithRouter(
      <DatabaseView tabId="t1" isActive={true} onConnectionSelected={() => {}} remoteOwner={null} onRemoteOwnerChange={() => {}} />
    );
    await waitFor(() => expect(screen.getByText("選擇資料庫連線")).toBeInTheDocument());
    expect(screen.getByText("My PG")).toBeInTheDocument();
  });

  it("shows sub-tabs after connection is provided", async () => {
    renderWithRouter(
      <DatabaseView tabId="t1" isActive={true} dbConnectionId="c1" onConnectionSelected={() => {}} remoteOwner={null} onRemoteOwnerChange={() => {}} />
    );
    await waitFor(() => expect(screen.getByText("瀏覽")).toBeInTheDocument());
    expect(screen.getByText("AI Chat")).toBeInTheDocument();
    expect(screen.getAllByText("SQL Editor").length).toBeGreaterThan(0);
  });
});
