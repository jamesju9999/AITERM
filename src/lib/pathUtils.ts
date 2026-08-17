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

/**
 * LRM（Left-to-Right Mark，U+200E）字元。
 *
 * 首頁用 `direction: rtl` 搭配 `text-overflow: ellipsis` 讓長路徑從開頭省略、
 * 保留尾端（見 `.home-resume-cwd` / `.home-recent-parent`）。但這個技巧有副
 * 作用：路徑開頭的中性符號（"/"、"~"）在 bidi 演算法下沒有前一個強方向字元
 * 可依附，會被判成跟隨段落方向（RTL），因而被排到視覺尾端——連「沒有被省略
 * 的短路徑」都會中獎，例如 "~/Downloads" 會顯示成 "Downloads/~"、
 * "/Users/x/Downloads/foo" 會顯示成 "…s/Downloads/foo/"（開頭的 "/" 跑到
 * 結尾）。已用無頭 Chrome 截圖實測重現。
 *
 * 修法：把整串文字前後都包上不可見的 LRM，強制被當成一段 LTR run 處理。
 * 同樣用無頭 Chrome 截圖驗證過——**必要的是「開頭」那個**（給開頭的中性符
 * 號一個明確的 LTR 錨點；只加尾端無效，實測過）；尾端那個目前的路徑資料裡
 * 從沒用到（我們的路徑不會以中性符號結尾），但保留它讓寫法對稱、也不會有
 * 副作用，用兩端包住比較不容易被未來的資料格式踩到同一個坑。
 */
export const LRM = "‎";

/** 把文字前後包上 LRM，修正 `direction: rtl` 容器裡開頭符號跑位的問題。
 *  見上面 `LRM` 常數的說明。 */
export function withLrmGuard(text: string): string {
  return LRM + text + LRM;
}

/** Split an absolute path into its final segment ("資料夾名") and everything
 *  before it ("父路徑", no trailing separator). Both are normalized to "/".
 *  Used by the home page's recent-projects list to put the project name up front. */
export function splitPathTail(p: string): { name: string; parent: string } {
  const unified = p.replace(/\\/g, "/").replace(/\/+$/, "");
  const idx = unified.lastIndexOf("/");
  if (idx < 0) return { name: unified, parent: "" };
  return { name: unified.slice(idx + 1), parent: idx === 0 ? "/" : unified.slice(0, idx) };
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
