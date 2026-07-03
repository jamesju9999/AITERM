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
