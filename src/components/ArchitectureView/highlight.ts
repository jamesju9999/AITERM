import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import cpp from "highlight.js/lib/languages/cpp";
import csharp from "highlight.js/lib/languages/csharp";
import css from "highlight.js/lib/languages/css";
import go from "highlight.js/lib/languages/go";
import ini from "highlight.js/lib/languages/ini";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import kotlin from "highlight.js/lib/languages/kotlin";
import markdown from "highlight.js/lib/languages/markdown";
import php from "highlight.js/lib/languages/php";
import python from "highlight.js/lib/languages/python";
import ruby from "highlight.js/lib/languages/ruby";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import swift from "highlight.js/lib/languages/swift";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

const LANGS = { bash, cpp, csharp, css, go, ini, java, javascript, json, kotlin, markdown, php, python, ruby, rust, sql, swift, typescript, xml, yaml };
for (const [name, def] of Object.entries(LANGS)) hljs.registerLanguage(name, def);

const BY_EXT: Record<string, keyof typeof LANGS> = {
  java: "java", kt: "kotlin", kts: "kotlin", ts: "typescript", tsx: "typescript", mts: "typescript", cts: "typescript",
  js: "javascript", jsx: "javascript", mjs: "javascript", cjs: "javascript", py: "python", rs: "rust", go: "go",
  c: "cpp", h: "cpp", cc: "cpp", cpp: "cpp", hpp: "cpp", cs: "csharp", php: "php", rb: "ruby", swift: "swift",
  sql: "sql", json: "json", xml: "xml", html: "xml", htm: "xml", svg: "xml", xsl: "xml", xsd: "xml", jsp: "xml",
  css: "css", scss: "css", yml: "yaml", yaml: "yaml", md: "markdown", sh: "bash", bash: "bash", zsh: "bash",
  toml: "ini", ini: "ini", properties: "ini", cfg: "ini",
};

/** 依副檔名決定語言；沒有副檔名或不認得回 null。 */
export function languageFor(path: string): string | null {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  if (dot < 0) return null;
  return BY_EXT[name.slice(dot + 1).toLowerCase()] ?? null;
}

/**
 * 把 highlight.js 輸出的 HTML 依換行切成多行，每行自成一體：
 * 跨行的 <span> 在換行處關閉，下一行再用同樣的 class 重開（多行註解、字串才能每行都有顏色）。
 */
export function splitHtmlLines(html: string): string[] {
  const lines: string[] = [];
  const open: string[] = [];
  let cur = "";
  const re = /<span[^>]*>|<\/span>|\n/g;
  let last = 0;
  for (let m = re.exec(html); m; m = re.exec(html)) {
    cur += html.slice(last, m.index);
    last = m.index + m[0].length;
    if (m[0] === "\n") {
      lines.push(cur + "</span>".repeat(open.length));
      cur = open.join("");
    } else if (m[0] === "</span>") {
      open.pop();
      cur += m[0];
    } else {
      open.push(m[0]);
      cur += m[0];
    }
  }
  lines.push(cur + html.slice(last));
  return lines;
}

/** 回傳每行一段已跳脫的 HTML（可直接 dangerouslySetInnerHTML）；不支援的語言或失敗回 null。 */
export function highlightLines(source: string, path: string): string[] | null {
  const language = languageFor(path);
  if (!language) return null;
  try {
    return splitHtmlLines(hljs.highlight(source, { language, ignoreIllegals: true }).value);
  } catch {
    return null;
  }
}
