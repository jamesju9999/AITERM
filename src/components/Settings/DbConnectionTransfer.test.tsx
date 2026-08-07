import { describe, it, expect, vi, beforeEach } from "vitest";
import { translateDbTransferError } from "./DbConnectionTransfer";
import { translations } from "../../lib/i18n";

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
