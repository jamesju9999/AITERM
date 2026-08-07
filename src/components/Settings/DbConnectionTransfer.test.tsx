import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { LocaleProvider } from "../../contexts/LocaleContext";
import { translateDbTransferError, DbExportPanel, DbImportPanel } from "./DbConnectionTransfer";
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

import { save, open } from "@tauri-apps/plugin-dialog";
import { dbExportConnections, dbCheckImportFile, dbPreviewImport, dbImportConnections } from "../../ipc/db";
import type { ImportPreviewItem } from "../../ipc/db";

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

const PREVIEW: ImportPreviewItem[] = [
  { id: "a", name: "總行LBOTHODB", db_type: "db2", host: "172.19.2.83", port: 25000,
    database: "LBOTHODB", username: "nuntio", conflict: "overwrite", existing_name: "舊的總行" },
  { id: "z", name: "新連線", db_type: "mysql", host: "10.0.0.5", port: 3306,
    database: "app", username: "root", conflict: "new", existing_name: null },
];

function renderImport(onDone = vi.fn(), onClose = vi.fn()) {
  render(
    <LocaleProvider>
      <DbImportPanel onClose={onClose} onDone={onDone} />
    </LocaleProvider>,
  );
  return { onDone, onClose };
}

describe("DbImportPanel", () => {
  it("closes without any IPC when the open dialog is cancelled", async () => {
    vi.mocked(open).mockResolvedValue(null);
    const { onClose } = renderImport();
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(dbCheckImportFile).not.toHaveBeenCalled();
  });

  it("shows the error and no passphrase field when the file is rejected", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/bad.json");
    vi.mocked(dbCheckImportFile).mockRejectedValue("unsupported_version");
    renderImport();

    await waitFor(() =>
      expect(
        screen.getByText("此檔案由較新版本的 AITerm 匯出，請先更新 AITerm"),
      ).toBeInTheDocument(),
    );
    // 使用者不該為一個注定被拒的檔案白打一次密碼
    expect(screen.queryByLabelText("加密密碼")).not.toBeInTheDocument();
    expect(dbPreviewImport).not.toHaveBeenCalled();
  });

  it("asks for the passphrase once the file passes the header check", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    renderImport();
    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
  });

  it("renders new and overwrite labels in the preview", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    vi.mocked(dbPreviewImport).mockResolvedValue(PREVIEW);
    renderImport();

    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: "pw" } });
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    await waitFor(() => expect(screen.getByText("覆蓋（原：舊的總行）")).toBeInTheDocument());
    expect(screen.getByText("新增")).toBeInTheDocument();
  });

  it("shows a localized message for a wrong passphrase", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    vi.mocked(dbPreviewImport).mockRejectedValue("wrong_passphrase");
    renderImport();

    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: "nope" } });
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    await waitFor(() =>
      expect(screen.getByText("加密密碼錯誤，或檔案已損毀")).toBeInTheDocument(),
    );
  });

  it("imports only the checked ids and reports the counts", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    vi.mocked(dbPreviewImport).mockResolvedValue(PREVIEW);
    vi.mocked(dbImportConnections).mockResolvedValue({ added: 1, overwritten: 0, failures: [] });
    const { onDone } = renderImport();

    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: "pw" } });
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));

    await waitFor(() => expect(screen.getAllByRole("checkbox")).toHaveLength(2));
    fireEvent.click(screen.getAllByRole("checkbox")[0]); // 取消勾選覆蓋那筆
    fireEvent.click(screen.getByRole("button", { name: "匯入" }));

    await waitFor(() =>
      expect(dbImportConnections).toHaveBeenCalledWith("/tmp/ok.json", "pw", ["z"]),
    );
    await waitFor(() => expect(onDone).toHaveBeenCalledWith("新增 1 筆、覆蓋 0 筆"));
  });

  it("lists per-item failures alongside the summary", async () => {
    vi.mocked(open).mockResolvedValue("/tmp/ok.json");
    vi.mocked(dbCheckImportFile).mockResolvedValue(1);
    vi.mocked(dbPreviewImport).mockResolvedValue(PREVIEW);
    vi.mocked(dbImportConnections).mockResolvedValue({
      added: 1, overwritten: 1,
      failures: [{ name: "總行LBOTHODB", reason: "secret_write_failed: denied" }],
    });
    renderImport();

    await waitFor(() => expect(screen.getByLabelText("加密密碼")).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText("加密密碼"), { target: { value: "pw" } });
    fireEvent.click(screen.getByRole("button", { name: "下一步" }));
    await waitFor(() => expect(screen.getAllByRole("checkbox")).toHaveLength(2));
    fireEvent.click(screen.getByRole("button", { name: "匯入" }));

    await waitFor(() =>
      expect(screen.getByText(/secret_write_failed: denied/)).toBeInTheDocument(),
    );
  });
});
