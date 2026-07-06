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
  return targets.some(t => !isPathInside(resolveAgainstRoot(t, root), root));
}

function resolveAgainstRoot(p: string, root: string): string {
  const isAbsolute = /^\//.test(p) || /^[a-zA-Z]:[\\/]/.test(p);
  return isAbsolute ? p : `${root}/${p}`;
}

function stripQuotes(s: string): string {
  if ((s.startsWith('"') && s.endsWith('"')) || (s.startsWith("'") && s.endsWith("'"))) {
    return s.slice(1, -1);
  }
  return s;
}

/** Split a shell command into top-level segments on ; && || | and newlines (not quote-aware — best effort). */
function splitSegments(command: string): string[] {
  return command.split(/;|&&|\|\||\n|(?<!\d)\|(?!\|)/).map(s => s.trim()).filter(Boolean);
}

/** Extract candidate file-write target paths from a shell command string. Best-effort, not exhaustive. */
function extractWriteTargets(command: string): string[] {
  const targets: string[] = [];

  // Redirection: `>` or `>>`, optionally preceded by a fd number (e.g. `2>`).
  // Excludes fd-duplication forms like `2>&1` or `>&2` (no path involved).
  const redirectRe = /\d*>{1,2}(?!&)\s*(\S+)/g;
  let m: RegExpExecArray | null;
  while ((m = redirectRe.exec(command)) !== null) {
    targets.push(stripQuotes(m[1]));
  }

  for (const segment of splitSegments(command)) {
    const words = segment.split(/\s+/).filter(Boolean);
    if (words.length === 0) continue;
    const cmdName = words[0];

    if (cmdName === "tee") {
      const arg = words.slice(1).find(w => !w.startsWith("-"));
      if (arg) targets.push(stripQuotes(arg));
    }

    if (cmdName === "cp" || cmdName === "mv") {
      const nonFlags = words.slice(1).filter(w => !w.startsWith("-"));
      if (nonFlags.length > 0) targets.push(stripQuotes(nonFlags[nonFlags.length - 1]));
    }

    if (cmdName === "dd") {
      const ofArg = words.find(w => w.startsWith("of="));
      if (ofArg) targets.push(stripQuotes(ofArg.slice(3)));
    }

    if (cmdName === "sed" && words.some(w => w === "-i" || w.startsWith("-i"))) {
      const nonFlags = words.slice(1).filter(w => !w.startsWith("-"));
      if (nonFlags.length > 0) targets.push(stripQuotes(nonFlags[nonFlags.length - 1]));
    }
  }

  return targets;
}
