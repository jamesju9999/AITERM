/**
 * 沒有 OSC 133 的 shell（ssh 進 NAS、busybox ash 等）不會送「指令結束」訊號，
 * agent 迴圈因此永遠等不到完成。後備判斷：畫面安靜一段時間、而且最後一行
 * 長得像提示字元，就當作指令已經跑完。見
 * docs/superpowers/specs/2026-10-07-prompt-heuristic-completion-design.md。
 */

// eslint-disable-next-line no-control-regex
const ANSI = /\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)/g;

export function lastNonEmptyLine(text: string): string {
  const lines = text.replace(ANSI, "").split(/\r?\n/);
  for (let i = lines.length - 1; i >= 0; i--) {
    if (lines[i].trim() !== "") return lines[i];
  }
  return "";
}

/**
 * 提示字元的特徵：最後一個非空白字元是常見的提示符號，而且**符號後面是行尾**
 * （通常還帶一個空白），前面還有一段 user@host／路徑之類的前綴。只有符號
 * 本身也算（例如 `$ `、`# `）。
 *
 * 刻意偏保守：誤判成「已完成」會在指令還在跑時就讓 agent 接手，比等到卡住
 * 偵測（120 秒）慢一點還糟，所以要求符號與前一個字元之間有空白或是
 * 常見的 user@host:path 形式，擋掉 `5$` 這種行內出現的符號。
 */
export function looksLikePrompt(line: string): boolean {
  if (line.trim() === "") return false;
  // oh-my-zsh 的預設樣式以 ➜ 開頭、結尾是 git 狀態符號而不是提示符號。
  if (/^➜\s/.test(line)) return true;
  // 提示符號必須落在行尾（允許尾端空白）。
  const m = line.match(/^(.*?)([$#%>❯»])\s*$/);
  if (!m) return false;
  const prefix = m[1];
  // 光是 "$ " 或 "# "。裸的 "> " 是 PS2（heredoc 續行）不是提示字元。
  if (prefix === "") return m[2] !== ">";
  // 符號前要有空白（"[~] # "、"proj % "），或是 user@host:path 這種形式。
  if (/\s$/.test(prefix)) return true;
  if (/^[^\s@]+@[^\s:]+(?::\S*)?$/.test(prefix)) return true;
  // 沒設 PS1 的 bash／sh 預設提示字元："sh-3.2$ "、"bash-5.1# "
  if (/^(?:sh|bash|zsh|ash|dash|ksh)-[\d.]+$/.test(prefix)) return true;
  // PowerShell: "PS C:\\path>"
  if (m[2] === ">" && /^PS\s/.test(prefix)) return true;
  return false;
}
