/** Normalize separators to "/" and resolve "." / ".." segments (no filesystem access). */
export function normalizePath(p: string): string {
  const unified = p.replace(/\\/g, "/");
  const isAbsolute = unified.startsWith("/");
  const out: string[] = [];
  for (const part of unified.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") {
      // A drive-letter first segment ("C:") is a root token — ".." can never pop it,
      // same as the leading "/" of absolute POSIX paths.
      if (
        out.length > 0 &&
        out[out.length - 1] !== ".." &&
        !(out.length === 1 && /^[a-zA-Z]:$/.test(out[0]))
      ) {
        out.pop();
      }
      continue;
    }
    out.push(part);
  }
  return (isAbsolute ? "/" : "") + out.join("/");
}

/** True if `child` resolves to a location at or under `root`. Windows drive paths compare case-insensitively. */
export function isPathInside(child: string, root: string): boolean {
  let c = normalizePath(child);
  let r = normalizePath(root);
  // Windows drive-letter paths (e.g. "C:/...") are case-insensitive
  const isWindowsPath = /^[a-zA-Z]:\//.test(c) || /^[a-zA-Z]:\//.test(r);
  if (isWindowsPath) {
    c = c.toLowerCase();
    r = r.toLowerCase();
  }
  return c === r || c.startsWith(r.endsWith("/") ? r : r + "/");
}
