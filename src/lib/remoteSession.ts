/**
 * 終端機分頁裡「ssh／telnet 進去之後」的遠端工作階段偵測。
 *
 * 那個分頁的 shell 其實一直是同一條 PTY——ssh 連上之後，之後打的每個指令都在遠端
 * 執行，不需要（也不應該）再連一次。問題是 AI 不知道這件事：它的提示詞只有本機的
 * 工作目錄，於是會在遠端 shell 裡又重打一次 `sshpass ... ssh`。這裡負責從使用者／AI
 * 送出的指令推斷「現在人在哪台遠端機器」，給提示詞用。
 */

/** 帶參數值的 ssh 選項：後面那個詞是選項的值，不是主機。 */
const SSH_OPTS_WITH_ARG = new Set([
  "-p", "-o", "-i", "-l", "-F", "-J", "-L", "-R", "-D", "-b", "-c", "-E", "-e",
  "-I", "-m", "-O", "-Q", "-S", "-w", "-W", "-B", "-P",
]);

/** 簡單的 shell 斷詞：認得單雙引號，其餘以空白切。夠用，不是完整的 shell 語法。 */
function tokenize(cmd: string): string[] {
  const out: string[] = [];
  const re = /"((?:\\.|[^"\\])*)"|'([^']*)'|(\S+)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(cmd)) !== null) out.push(m[1] ?? m[2] ?? m[3]);
  return out;
}

const HOST_RE = /^(?:[\w.-]+@)?[\w.-]+$/;

export interface RemoteLogin {
  target: string;
}

/**
 * 這個指令是「開一個互動式遠端 shell」嗎？是的話回傳目標（user@host 或 host）。
 * `ssh host 'cmd'` 這種單次指令不算——它跑完就斷，不會留下遠端 shell。
 */
export function parseRemoteLogin(command: string): RemoteLogin | null {
  const tokens = tokenize(command.trim());
  // 去掉開頭的 VAR=value 環境變數（例如 SSHPASS='pw' sshpass -e ssh ...）。
  let i = 0;
  while (i < tokens.length && /^[A-Za-z_]\w*=/.test(tokens[i])) i++;
  let prog = tokens[i];
  if (!prog) return null;

  if (prog === "sshpass") {
    // sshpass [-p pw | -e | -f file | -d fd | -P prompt] ssh ...
    i++;
    while (i < tokens.length && tokens[i] !== "ssh" && tokens[i] !== "telnet") {
      i += ["-p", "-f", "-d", "-P"].includes(tokens[i]) ? 2 : 1;
    }
    prog = tokens[i];
    if (!prog) return null;
  }

  if (prog === "telnet") {
    const host = tokens.slice(i + 1).find((t) => !t.startsWith("-"));
    return host && HOST_RE.test(host) ? { target: host } : null;
  }

  if (prog === "ssh" || prog === "mosh") {
    let j = i + 1;
    while (j < tokens.length) {
      const t = tokens[j];
      if (t.startsWith("-")) {
        // 轉送埠／不開 shell 的選項：不是互動式登入。
        if (t === "-N" || t === "-W" || t === "-L" || t === "-R" || t === "-D") return null;
        j += SSH_OPTS_WITH_ARG.has(t) ? 2 : 1;
        continue;
      }
      break;
    }
    const host = tokens[j];
    if (!host || !HOST_RE.test(host)) return null;
    // 主機後面還有東西＝要在遠端跑的單次指令。
    if (j + 1 < tokens.length) return null;
    return { target: host };
  }
  return null;
}

export function isLogoutCommand(command: string): boolean {
  return /^(?:exit|logout)(?:\s+\d+)?$/.test(command.trim());
}

/** 給 AI 提示詞的一段話。沒有遠端工作階段時回傳空字串。 */
export function remoteSessionPromptNote(target: string | null | undefined): string {
  if (!target) return "";
  return `

IMPORTANT — remote session: this terminal is currently INSIDE an interactive remote shell on "${target}" (opened earlier with ssh/telnet). Every command you run executes on that remote machine, in that same session. Therefore:
- Do NOT run ssh, sshpass, telnet or mosh to reach that machine again — you are already there. Just run the command directly.
- The working directory and directory listing above describe the LOCAL machine and may not apply here; use pwd / ls to look at the remote side if you need to.
- Tools that exist only on the local machine (for example sshpass, or local files) are not available inside the session. To go back to the local machine, run <cmd>exit</cmd> first.
- The remote shell may not report exit codes; judge success from the output.`;
}
