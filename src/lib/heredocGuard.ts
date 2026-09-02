/**
 * 補上 AI 產生的指令裡缺掉的 heredoc 結束標記。
 *
 * 為什麼需要：agent 會把模型給的指令直接寫進互動式 shell。實測（真的開一個 PTY
 * 跑 zsh 驗證過）確認——格式正確的 heredoc 完全沒問題，但只要結束標記沒有單獨
 * 佔一行（例如模型寫成 `EOF echo done`，或整段被壓成單行），shell 就停在
 * `heredoc>` 等一個永遠不會來的標記。指令沒結束 ⇒ OSC 133 D 不會發出 ⇒ agent
 * 的區塊永遠是 running ⇒ 整個迴圈就在那裡等，只有兩分鐘後的「似乎沒有反應」
 * 提示能救，而且要人來按。
 *
 * 補一行結束標記不會讓內容變正確，但會讓指令**跑得完**：agent 因此看得到結果、
 * 可以自己重試，而不是整條迴圈掛死。
 *
 * 只用在 agent 自動執行的路徑上——使用者自己在終端機打 heredoc 時，停在
 * `heredoc>` 繼續輸入正是他要的，不可以雞婆。
 */

/** `<<` 或 `<<-` 後面接引號或不接的分界字。`<<<`（herestring）要排除。 */
const OPENER = /<<(-?)(?!<)\s*(?:'([A-Za-z_][\w]*)'|"([A-Za-z_][\w]*)"|([A-Za-z_][\w]*))/g;

export function repairUnterminatedHeredocs(cmd: string): string {
  const lines = cmd.split("\n");
  // 分界字只在第一行的開頭段落宣告；後面的行是內容，裡面出現的 << 不算。
  const openers: { delimiter: string; allowIndent: boolean }[] = [];
  let m: RegExpExecArray | null;
  OPENER.lastIndex = 0;
  while ((m = OPENER.exec(lines[0] ?? "")) !== null) {
    const delimiter = m[2] ?? m[3] ?? m[4];
    if (delimiter) openers.push({ delimiter, allowIndent: m[1] === "-" });
  }
  if (openers.length === 0) return cmd;

  // 每個分界字要在「之後的某一行」單獨出現才算已終止。依開啟順序逐一往下找，
  // 一個結束標記只能配一個 heredoc。
  const missing: string[] = [];
  let cursor = 1;
  for (const { delimiter, allowIndent } of openers) {
    let found = -1;
    for (let i = cursor; i < lines.length; i++) {
      const line = allowIndent ? lines[i].replace(/^\t+/, "") : lines[i];
      if (line === delimiter) { found = i; break; }
    }
    if (found === -1) {
      missing.push(delimiter);
    } else {
      cursor = found + 1;
    }
  }
  if (missing.length === 0) return cmd;

  return `${cmd}\n${missing.join("\n")}`;
}
