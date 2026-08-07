import type { Translations } from "../../lib/i18n";

/** 後端 `ImportError` 的變體名稱 → i18n key。這些字串是介面契約的一部分，
 *  改動時要同步 `src-tauri/src/commands/db_export.rs` 的 `#[error(...)]`。 */
const ERROR_KEYS = {
  not_an_export_file: "db_err_not_an_export_file",
  unsupported_version: "db_err_unsupported_version",
  wrong_passphrase: "db_err_wrong_passphrase",
  unsupported_kdf: "db_err_unsupported_kdf",
} as const;

/** 已知錯誤碼轉成本地化訊息；其餘（例如 `io_error: ...`）原樣顯示。 */
export function translateDbTransferError(t: Translations, e: unknown): string {
  const raw = String(e);
  const key = ERROR_KEYS[raw as keyof typeof ERROR_KEYS];
  return key ? (t[key] as string) : raw;
}
