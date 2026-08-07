import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translateDbTransferError, DbExportPanel } from "./DbConnectionTransfer";
import { translations } from "../../lib/i18n";
import type { DbConnectionInfo } from "../../ipc/db";

vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn(), open: vi.fn() }));
vi.mock("../../ipc/db", () => ({
  dbCheckImportFile: vi.fn(),
  dbExportConnections: vi.fn(),
  dbPreviewImport: vi.fn(),
  dbImportConnections: vi.fn(),
  DB_TYPE_LABELS: { postgresql: "PostgreSQL", mysql: "MySQL", sqlite: "SQLite", mssql: "MSSQL", db2: "DB2" },
}));

import { save } from "@tauri-apps/plugin-dialog";
import { dbExportConnections } from "../../ipc/db";

const CONNS: DbConnectionInfo[] = [
  { id: "a", name: "總行LBOTHODB", db_type: "db2", host: "172.19.2.83", port: 25000,
    database: "LBOTHODB", username: "nuntio", default_schema: "NUNTIO", is_connected: true },
  { id: "b", name: "MSSQL-Docker", db_type: "mssql", host: "192.168.1.30", port: 1433,
    database: "master", username: "sa", default_schema: null, is_connected: false },
];

beforeEach(() => {
  vi.clearAllMocks();
  Object.defineProperty(window, "localStorage", {
    value: { getItem: vi.fn(() => null), setItem: vi.fn(), removeItem: vi.fn(), clear: vi.fn(), key: vi.fn(), length: 0 },
    writable: true,
  });
});

function renderExport(onDone = vi.fn(), onClose = vi.fn()) {
  render(
    <LocaleProvider>
      <DbExportPanel connections={CONNS} onClose={onClose} onDone={onDone} />
    </LocaleProvider>,
  );
  return { onDone, onClose };
}

function typePassphrases(pw: string, confirm: string) {
  fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: pw } });
  fireEvent.change(screen.getByLabelText("確認加密密碼"), { target: { value: confirm } });
}

const t = translations["zh-TW"];

describe("translateDbTransferError", () => {
  it("maps a known error code to its localized message", () => {
    expect(translateDbTransferError(t, "wrong_passphrase")).toBe(t.db_err_wrong_passphrase);
    expect(translateDbTransferError(t, "not_an_export_file")).toBe(t.db_err_not_an_export_file);
    expect(translateDbTransferError(t, "unsupported_version")).toBe(t.db_err_unsupported_version);
    expect(translateDbTransferError(t, "unsupported_kdf")).toBe(t.db_err_unsupported_kdf);
  });

  it("falls back to the raw text for unknown errors", () => {
    expect(translateDbTransferError(t, "io_error: no such file")).toBe("io_error: no such file");
  });

  it("stringifies non-string rejections", () => {
    expect(translateDbTransferError(t, new Error("boom"))).toContain("boom");
  });
});

describe("DbExportPanel", () => {
  it("checks every connection by default", () => {
    renderExport();
    const boxes = screen.getAllByRole("checkbox");
    expect(boxes).toHaveLength(2);
    boxes.forEach((b) => expect(b).toBeChecked());
  });

  it("disables export until both passphrases match", () => {
    renderExport();
    const btn = screen.getByRole("button", { name: "匯出" });
    expect(btn).toBeDisabled();

    typePassphrases("hunter2", "hunter3");
    expect(btn).toBeDisabled();
    expect(screen.getByText("兩次輸入的加密密碼不一致")).toBeInTheDocument();

    typePassphrases("hunter2", "hunter2");
    expect(btn).toBeEnabled();
  });

  it("disables export when nothing is selected", () => {
    renderExport();
    typePassphrases("pw", "pw");
    screen.getAllByRole("checkbox").forEach((b) => fireEvent.click(b));
    expect(screen.getByRole("button", { name: "匯出" })).toBeDisabled();
  });

  it("sends only the checked ids", async () => {
    vi.mocked(save).mockResolvedValue("/tmp/out.json");
    vi.mocked(dbExportConnections).mockResolvedValue(1);
    const { onDone } = renderExport();

    typePassphrases("pw", "pw");
    fireEvent.click(screen.getAllByRole("checkbox")[1]); // 取消勾選 MSSQL-Docker
    fireEvent.click(screen.getByRole("button", { name: "匯出" }));

    await waitFor(() =>
      expect(dbExportConnections).toHaveBeenCalledWith("/tmp/out.json", ["a"], "pw"),
    );
    await waitFor(() => expect(onDone).toHaveBeenCalledWith("已匯出 1 筆連線"));
  });

  it("does not call the backend when the save dialog is cancelled", async () => {
    vi.mocked(save).mockResolvedValue(null);
    renderExport();
    typePassphrases("pw", "pw");
    fireEvent.click(screen.getByRole("button", { name: "匯出" }));
    await waitFor(() => expect(save).toHaveBeenCalled());
    expect(dbExportConnections).not.toHaveBeenCalled();
  });

  it("shows a localized message when the backend fails", async () => {
    vi.mocked(save).mockResolvedValue("/tmp/out.json");
    vi.mocked(dbExportConnections).mockRejectedValue("io_error: disk full");
    renderExport();
    typePassphrases("pw", "pw");
    fireEvent.click(screen.getByRole("button", { name: "匯出" }));
    await waitFor(() => expect(screen.getByText("io_error: disk full")).toBeInTheDocument());
  });
});
