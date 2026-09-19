import type { LaunchRequest } from "../ipc/launch";

export type LaunchPlan =
  | { kind: "open"; cwd?: string; command?: string }
  | { kind: "confirm-script"; cwd?: string; scriptPath: string; command: string };

const SAFE_POSIX = /^[A-Za-z0-9_/.:=@%+,-]+$/;
const SAFE_WINDOWS = /^[A-Za-z0-9_\\/.:=@%+,-]+$/;

/** 把單一參數變成 shell 安全的字串。POSIX 用單引號，Windows 用雙引號。 */
export function quoteArg(arg: string, isWindows: boolean): string {
  if (isWindows) {
    return SAFE_WINDOWS.test(arg) ? arg : `"${arg.replace(/"/g, '\\"')}"`;
  }
  return SAFE_POSIX.test(arg) ? arg : `'${arg.replace(/'/g, "'\\''")}'`;
}

/**
 * 決定一個啟動請求要怎麼處理：
 * - `-e` 指令：直接開分頁並在 shell 就緒後送出。
 * - 腳本：要先讓使用者確認（雙擊 `.command` 等於執行任意程式）。
 *   Windows 沒有註冊這個入口，且 `.sh` 在那邊無法直接執行，所以只開在該目錄。
 */
export function planLaunch(req: LaunchRequest, isWindows: boolean): LaunchPlan {
  const cwd = req.cwd ?? undefined;
  const base = cwd === undefined ? {} : { cwd };

  if (req.command && req.command.length > 0) {
    return { kind: "open", ...base, command: req.command.map((a) => quoteArg(a, isWindows)).join(" ") };
  }
  if (req.script) {
    if (isWindows) return { kind: "open", ...base };
    return { kind: "confirm-script", ...base, scriptPath: req.script, command: quoteArg(req.script, false) };
  }
  return { kind: "open", ...base };
}
