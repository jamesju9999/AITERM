/**
 * Parse a line of user input for the `/ai ` prefix. Returns the trimmed query
 * text when present, `null` when the line does not begin with `/ai ` or has no
 * query. Only lowercase `/ai` is recognized.
 */
export function parseAiPrefix(line: string): string | null {
  if (!line.startsWith("/ai")) return null;
  // Must be followed by at least one whitespace character; otherwise it's a
  // token like `/airplane` or just `/ai` alone.
  if (line.length === 3) return null;
  const next = line.charAt(3);
  if (next !== " " && next !== "\t") return null;
  const rest = line.slice(3).trim();
  return rest.length === 0 ? null : rest;
}
