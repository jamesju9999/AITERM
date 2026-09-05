import { invoke } from "@tauri-apps/api/core";

/** 鏡射 Rust 的 `commands::reports::ReportInfo`。 */
export interface ReportInfo {
  filename: string;
  /** Unix 秒。 */
  saved_at: number;
  /** 從 HTML 的 `<title>` 取出；null 時前端顯示檔名。 */
  title: string | null;
}

/** 存進 `<專案>/reports/`，回傳實際使用的檔名。 */
export const saveReport = (projectId: string, html: string): Promise<string> =>
  invoke("reports_save", { projectId, html });

/** 該專案的歷史報告，新到舊。 */
export const listReports = (projectId: string): Promise<ReportInfo[]> =>
  invoke("reports_list", { projectId });

export const readReport = (projectId: string, filename: string): Promise<string> =>
  invoke("reports_read", { projectId, filename });
