import type { ReactNode } from "react";

/**
 * If `raw` looks like a JSON object with a common response field, extract it.
 * Handles:
 *   - Direct JSON: { "response": "..." }
 *   - Fenced JSON: ```json\n{ "response": "..." }\n```
 * Also handles partial/streaming JSON gracefully.
 */
export function extractResponseText(raw: string): string {
  let trimmed = raw.trim();

  // Strip markdown code fences: ```json\n...\n``` or ```\n...\n```
  if (trimmed.startsWith("```")) {
    const newlinePos = trimmed.indexOf("\n");
    if (newlinePos !== -1) {
      const inner = trimmed.slice(newlinePos + 1);
      const closePos = inner.lastIndexOf("```");
      trimmed = closePos !== -1 ? inner.slice(0, closePos).trim() : inner.trim();
    }
  }

  if (!trimmed.startsWith("{")) return raw;

  // Full parse first
  try {
    const parsed = JSON.parse(trimmed);
    if (parsed && typeof parsed === "object") {
      for (const key of ["response", "text", "content", "answer", "output"]) {
        if (typeof parsed[key] === "string") return parsed[key];
      }
    }
  } catch {
    // Partial JSON (mid-stream) — regex fallback
    const m = trimmed.match(
      /"(?:response|text|content|answer|output)"\s*:\s*"([\s\S]*?)(?:(?<!\\)"\s*[,}]|$)/
    );
    if (m) {
      try {
        return JSON.parse(`"${m[1]}"`);
      } catch {
        return m[1];
      }
    }
  }

  return raw;
}

/**
 * Convert literal \n / \t escape sequences to real characters.
 * Models sometimes emit these as plain text rather than actual control chars.
 */
export function unescapeNewlines(text: string): string {
  return text.replace(/\\n/g, "\n").replace(/\\t/g, "\t");
}

// ── Inline markdown ────────────────────────────────────────────────────────

type InlineNode =
  | string
  | { type: "bold" | "italic" | "code"; text: string };

function parseInline(text: string): InlineNode[] {
  const nodes: InlineNode[] = [];
  // **bold** first (before *italic*), then `code`, then *italic*
  const re = /(\*\*([^*\n]+)\*\*|`([^`\n]+)`|\*([^*\n]+)\*)/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) nodes.push(text.slice(last, m.index));
    if (m[2] !== undefined) nodes.push({ type: "bold",   text: m[2] });
    else if (m[3] !== undefined) nodes.push({ type: "code",   text: m[3] });
    else if (m[4] !== undefined) nodes.push({ type: "italic", text: m[4] });
    last = m.index + m[0].length;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

function renderInline(text: string, baseKey: string): ReactNode[] {
  return parseInline(text).map((n, i) => {
    if (typeof n === "string") return n;
    const k = `${baseKey}-${i}`;
    if (n.type === "bold")   return <strong key={k}>{n.text}</strong>;
    if (n.type === "code")   return <code   key={k} className="md-code">{n.text}</code>;
    if (n.type === "italic") return <em     key={k}>{n.text}</em>;
  });
}

// ── Block markdown renderer ────────────────────────────────────────────────

export function MarkdownText({ text }: { text: string }): ReactNode {
  const lines = text.split("\n");
  const elements: ReactNode[] = [];
  let listItems: string[] = [];
  let k = 0;

  const flushList = () => {
    if (!listItems.length) return;
    const items = listItems.splice(0);
    elements.push(
      <ul key={k++} className="md-list">
        {items.map((item, i) => (
          <li key={i}>{renderInline(item, `li-${k}-${i}`)}</li>
        ))}
      </ul>
    );
  };

  for (const line of lines) {
    const h3 = line.match(/^###\s+(.*)/);
    const h2 = line.match(/^##\s+(.*)/);
    const h1 = line.match(/^#\s+(.*)/);

    if (h3) { flushList(); elements.push(<h3 key={k++} className="md-h3">{renderInline(h3[1], `h3-${k}`)}</h3>); continue; }
    if (h2) { flushList(); elements.push(<h2 key={k++} className="md-h2">{renderInline(h2[1], `h2-${k}`)}</h2>); continue; }
    if (h1) { flushList(); elements.push(<h1 key={k++} className="md-h1">{renderInline(h1[1], `h1-${k}`)}</h1>); continue; }

    const li = line.match(/^[-*]\s+(.*)/);
    if (li) { listItems.push(li[1]); continue; }

    flushList();

    if (line.trim() === "") {
      elements.push(<div key={k++} className="md-gap" />);
    } else {
      elements.push(<p key={k++} className="md-p">{renderInline(line, `p-${k}`)}</p>);
    }
  }

  flushList();
  return <>{elements}</>;
}
