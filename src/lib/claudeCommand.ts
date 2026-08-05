/**
 * 這一行指令是不是在啟動 Claude Code？
 *
 * 只認「第一個 token 的檔名剛好是 claude」。刻意不支援環境變數前綴
 * （`FOO=1 claude`）與 `npx claude`：前者罕見，後者不是 Claude Code 的
 * 安裝方式，而支援它們要把單純的字串比對變成 shell 語法解析。漏報的
 * 代價只是這次不提示，下次執行照樣有機會。
 */
export function isClaudeCommand(cmd: string): boolean {
  const first = cmd.trim().split(/\s+/)[0];
  if (!first) return false;
  // Windows 的路徑用反斜線，POSIX 用斜線——兩種都要切。
  const basename = first.split(/[/\\]/).pop();
  return basename === "claude";
}
