import type { LaunchRequest } from "../ipc/launch";

/** `confirm-script.command` 就是加過引號的 `scriptPath`（刻意重複，讓消費端可以直接把它交給終端機）。 */
export type LaunchPlan =
  | { kind: "open"; cwd?: string; command?: string }
  | { kind: "confirm-script"; cwd?: string; scriptPath: string; command: string };

// `=` 與 `%` 刻意不在安全集合內：zsh（macOS 預設）裡，字首 `=` 是指令路徑展開
// （`=ls` → `/bin/ls`，`=不存在` 會讓整行中止）、第一個字 `FOO=bar` 會變成環境變數
// 賦值、字首 `%` 是工作參照。加引號對其他 shell 無害。
const SAFE_POSIX = /^[A-Za-z0-9_/.:@+,-]+$/;

// C0 控制字元（含 \t \n \r）與 DEL。
const CONTROL_CHARS = /[\x00-\x1f\x7f]/; // eslint-disable-line no-control-regex

/** 把單一參數變成 POSIX shell 安全的字串（單引號；空字串也會被引成 `''`）。 */
export function quoteArg(arg: string): string {
  return SAFE_POSIX.test(arg) ? arg : `'${arg.replace(/'/g, "'\\''")}'`;
}

/**
 * 決定一個啟動請求要怎麼處理：
 * - `-e` 指令：直接開分頁並在 shell 就緒後送出。
 * - 腳本：要先讓使用者確認（雙擊 `.command` 等於執行任意程式）。
 *
 * 兩種情況下一律只開分頁、什麼都不送：
 * - Windows：PowerShell 與 cmd.exe 的引號規則各自不同且有風險（雙引號內的
 *   `$(...)` 在 PowerShell 會執行、`%VAR%`、`@x` 展開等），Windows 的終端機整合
 *   是另一份規格；因此指令與腳本的自動執行在這裡刻意延後，只開在該目錄。
 * - 任何指令參數或腳本路徑含控制字元：指令字串是被「打進」活的 pty，
 *   0x00–0x1f 與 0x7f 會在 shell 解析引號之前就被行編輯器處理（Tab 觸發補全、
 *   DEL 是倒退、^C 會取消這一行讓後面的字變成新指令、^D 等），所以問題不在
 *   引號寫法、引號也擋不住。換行與 \r 也一併拒絕（寧可失敗也不送）。
 */
export function planLaunch(req: LaunchRequest, isWindows: boolean): LaunchPlan {
  const cwd = req.cwd ?? undefined;
  const base = cwd === undefined ? {} : { cwd };

  if (isWindows) return { kind: "open", ...base };

  const sent = [...(req.command ?? []), ...(req.script ? [req.script] : [])];
  if (sent.some((s) => CONTROL_CHARS.test(s))) return { kind: "open", ...base };

  if (req.command && req.command.length > 0) {
    return { kind: "open", ...base, command: req.command.map(quoteArg).join(" ") };
  }
  if (req.script) {
    return { kind: "confirm-script", ...base, scriptPath: req.script, command: quoteArg(req.script) };
  }
  return { kind: "open", ...base };
}
