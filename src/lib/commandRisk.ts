import { isPathInside } from "./pathUtils";

export type CommandRisk = "normal" | "dangerous";

const DANGEROUS_PATTERNS: RegExp[] = [
  /\brm\s+-[a-z]*(rf|fr)[a-z]*\b/i,               // rm -rf / rm -fr（含 -vrf 等組合）
  /\bsudo\b/,
  /\b(curl|wget)\b[^|]*\|\s*(ba|z|da|fi)?sh\b/i,  // curl ... | sh / bash / zsh
  /\bgit\s+push\b.*(\s--force\b|\s-f\b)/i,
  /\bdd\s+if=/i,
  /\bmkfs(\.\w+)?\b/i,
  /\bchmod\b.*\b777\b/i,
  /\b(shutdown|reboot|poweroff)\b/i,
  /\bdel\s+\/[sq]/i,                              // Windows del /s /q
  /\bformat\s+[a-z]:/i,                           // Windows format d:
  /\bremove-item\b.*-(recurse|force)/i,           // PowerShell
  /\brd\s+\/s/i,                                  // Windows rd /s
];

export function classifyCommand(command: string): CommandRisk {
  return DANGEROUS_PATTERNS.some((p) => p.test(command)) ? "dangerous" : "normal";
}

/**
 * Best-effort detector for shell commands whose write target resolves outside `root`.
 * Catches the common file-writing shapes (redirection, tee, cp/mv destination, dd of=,
 * sed -i). Not a sandbox — command substitution, base64 pipelines, and other obfuscation
 * can still evade this; it exists to catch the common case classifyCommand's
 * catastrophic-pattern list doesn't cover.
 */
export function commandWritesOutsideRoot(command: string, root: string): boolean {
  const targets = extractWriteTargets(command);
  return targets.some(t => !isSafeDeviceTarget(t) && (looksUnresolvable(t) || !isPathInside(resolveAgainstRoot(t, root), root)));
}

function isSafeDeviceTarget(p: string): boolean {
  return p === "/dev/null" || p === "/dev/stdout" || p === "/dev/stderr" || /^nul$/i.test(p);
}

/** Variable/command-substitution targets can't be statically resolved — fail closed (treat as escaping root). */
function looksUnresolvable(p: string): boolean {
  return p.includes("$") || p.includes("`");
}

function resolveAgainstRoot(p: string, root: string): string {
  const isAbsolute = /^\//.test(p) || /^[a-zA-Z]:[\\/]/.test(p) || /^\\\\/.test(p);
  return isAbsolute ? p : `${root}/${p}`;
}

/** Split a shell command into top-level segments on ; && || | and newlines (not quote-aware — best effort). */
function splitSegments(command: string): string[] {
  return command.split(/;|&&|\|\||\n|\|(?!\|)/).map(s => s.trim()).filter(Boolean);
}

/** Read the next shell token starting at index i, respecting single/double quotes (quotes stripped). Returns [token, nextIndex] or null at end of string. */
function readToken(s: string, i: number): [string, number] | null {
  while (i < s.length && /\s/.test(s[i])) i++;
  if (i >= s.length) return null;
  let token = "";
  while (i < s.length && !/\s/.test(s[i])) {
    const c = s[i];
    if (c === '"' || c === "'") {
      const quote = c;
      i++;
      while (i < s.length && s[i] !== quote) { token += s[i]; i++; }
      i++; // skip closing quote (or end of string if unterminated)
    } else {
      token += c;
      i++;
    }
  }
  return [token, i];
}

function tokenizeSegment(segment: string): string[] {
  const tokens: string[] = [];
  let i = 0;
  for (;;) {
    const r = readToken(segment, i);
    if (!r) break;
    tokens.push(r[0]);
    i = r[1];
  }
  return tokens;
}

/** Extract candidate file-write target paths from a shell command string. Best-effort, not exhaustive. */
function extractWriteTargets(command: string): string[] {
  const targets: string[] = [];

  // Redirection: `>` or `>>`, optionally preceded by a fd number (e.g. `2>`).
  // Excludes fd-duplication forms like `2>&1` or `>&2` (no path involved).
  const redirectRe = /\d*>{1,2}(?!&)/g;
  let m: RegExpExecArray | null;
  while ((m = redirectRe.exec(command)) !== null) {
    const r = readToken(command, redirectRe.lastIndex);
    if (r) targets.push(r[0]);
  }

  for (const segment of splitSegments(command)) {
    const words = tokenizeSegment(segment);
    if (words.length === 0) continue;
    const cmdName = words[0];
    const isUnixFlag = (w: string) => w.startsWith("-");
    const isWindowsCopyCmd = cmdName === "copy" || cmdName === "move" || cmdName === "xcopy" || cmdName === "robocopy";
    // Windows copy-family flags commonly use "/" (e.g. /s, /q); "-" prefixed flags never denote a path either way.
    const isFlag = (w: string) => w.startsWith("-") || (isWindowsCopyCmd && w.startsWith("/"));

    if (cmdName === "tee") {
      const arg = words.slice(1).find(w => !isUnixFlag(w));
      if (arg) targets.push(arg);
    }

    if (cmdName === "cp" || cmdName === "mv" || isWindowsCopyCmd) {
      const nonFlags = words.slice(1).filter(w => !isFlag(w));
      if (nonFlags.length > 0) targets.push(nonFlags[nonFlags.length - 1]);
    }

    if (cmdName === "dd") {
      const ofArg = words.find(w => w.startsWith("of="));
      if (ofArg) targets.push(ofArg.slice(3));
    }

    if (cmdName === "sed" && words.some(w => w === "-i" || w.startsWith("-i"))) {
      const nonFlags = words.slice(1).filter(w => !isUnixFlag(w));
      if (nonFlags.length > 0) targets.push(nonFlags[nonFlags.length - 1]);
    }
  }

  return targets;
}
