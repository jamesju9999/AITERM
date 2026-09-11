import { useEffect, useState } from "react";
import type { Terminal } from "@xterm/xterm";

export interface ShellIdentity {
  /** 目前只會是 "PowerShell"——cmd.exe 的整合靠 PROMPT 環境變數，送不出這個序列。 */
  shell: string;
  /** PowerShell 專屬：`Desktop` 是 Windows PowerShell 5.1，`Core` 是 7.x。 */
  edition: string;
  version: string;
}

function parse(payload: string): ShellIdentity | null {
  const fields = new Map<string, string>();
  for (const part of payload.split(";")) {
    const eq = part.indexOf("=");
    if (eq > 0) fields.set(part.slice(0, eq), part.slice(eq + 1));
  }
  const shell = fields.get("shell");
  const edition = fields.get("edition");
  const version = fields.get("version");
  // 三個欄位缺一就整筆不採用：顯示錯的身分比不顯示更糟。
  if (!shell || !edition || !version) return null;
  return { shell, edition, version };
}

/**
 * 接收 shell 自己回報的身分（OSC 7000，由 aiterm-core 的 shell.rs 注入腳本送出）。
 *
 * 刻意跟 `useTerminalBlocks` 的 OSC 133 處理器分開：兩者用途無關，而且這個
 * hook 的生命週期單純得多——收到就記住，不需要任何 ref 橋接。
 */
export function useShellIdentity(term: Terminal | null): ShellIdentity | null {
  const [identity, setIdentity] = useState<ShellIdentity | null>(null);

  useEffect(() => {
    if (!term) return;
    const disposable = term.parser.registerOscHandler(7000, (data) => {
      const parsed = parse(data);
      if (parsed) {
        // 腳本每次畫提示字元都重送一份（見 shell.rs：那是為了避開「處理器
        // 還沒註冊、身分就已經送出」的 race）。內容沒變就回傳原本那個物件，
        // React 會直接跳過重繪——否則每跑一個指令都會讓整個分頁重繪一次。
        setIdentity((prev) =>
          prev && prev.shell === parsed.shell && prev.edition === parsed.edition && prev.version === parsed.version
            ? prev
            : parsed,
        );
      }
      // true＝這個序列已經被處理掉，不要再往下傳。
      return true;
    });
    return () => disposable.dispose();
  }, [term]);

  return identity;
}
