// Mermaid source sanitization for AI-generated diagrams (Knowledge Base, Code
// Assistant, chat). LLMs frequently emit flowchart labels containing characters
// the Mermaid lexer chokes on (parens, colons, braces, pipes, angle brackets),
// so we normalize punctuation and then unconditionally quote every node/edge
// label — quoting a label is always syntactically safe, which removes the whole
// class of "unquoted special character confuses the lexer" errors at once.

// Pass 1 — conservative sanitization.
export function sanitizeMermaid(code: string): string {
  let result = code
    // Full-width punctuation → ASCII
    .replace(/（/g, "(").replace(/）/g, ")")
    .replace(/【/g, "[").replace(/】/g, "]")
    .replace(/｛/g, "{").replace(/｝/g, "}")
    .replace(/＜/g, "<").replace(/＞/g, ">")
    .replace(/｜/g, "|")
    .replace(/，/g, ",").replace(/。/g, ".").replace(/、/g, ",")
    .replace(/：/g, ":").replace(/；/g, ";")
    // Chinese corner quotes and curly double quotes → ASCII SINGLE quotes.
    // Only CURLY doubles (“ ”) are normalized here, never straight " — a label
    // that is already correctly straight-double-quoted (["text"], {"text"}) must
    // be left intact, otherwise it gets re-wrapped into ugly ["'text'"] and, for
    // diamonds, mis-quoted. wrapQuoted still collapses any straight " that ends
    // up INSIDE a label we quote ourselves.
    .replace(/[「」『』“”]/g, "'");

  // <br/> → space: the ">" is lexed as LINK_ID inside node labels.
  result = result.replace(/<br\s*\/?>/gi, " ");

  // Blanket label quoting only applies to flowchart/graph syntax — other
  // diagram types (classDiagram, stateDiagram, sequenceDiagram, ...) use
  // [] {} for unrelated constructs and would be corrupted by this pass.
  if (/^\s*(flowchart|graph)\b/im.test(result)) {
    // Repair edge labels the model broke by putting the delimiter `|` (and
    // quotes / angle brackets) INSIDE the label text, e.g.
    // `B -->|"有 |"Bearer <key>"|"| C`. Runs first, anchored on the arrow, so it
    // rebuilds a clean single-quoted-free `|"…"|` before anything else.
    result = repairEdgeLabels(result);
    result = quoteBracketLabels(result);
    // Protect every already-quoted "..." span before the diamond/edge passes so
    // they can't reach INSIDE a quoted label. Without this, a `{id}` written
    // inside a rectangle label — e.g. ["...Key: oauth:google:{id}"] — was
    // matched by the diamond pass and rewritten to {"id"}, injecting a stray "
    // into the quoted string and producing a "got 'STR'" parse error (reported
    // from the Knowledge Base). quoteBracketLabels runs first so freshly-quoted
    // rectangle labels are protected too.
    const quoted: string[] = [];
    result = result.replace(/"[^"\n]*"/g, (m) => {
      // Neutralize angle brackets inside already-quoted labels too — `<x>` is
      // lexed as an HTML tag (TAGSTART) and breaks the parse even when quoted.
      quoted.push(m.replace(/</g, "＜").replace(/>/g, "＞"));
      return `${MASK}${quoted.length - 1}${MASK}`;
    });
    result = quoteDiamondLabels(result);
    result = quoteEdgeLabels(result);
    result = result.replace(new RegExp(`${MASK}(\\d+)${MASK}`, "g"), (_m, i) => quoted[Number(i)]);
  }

  result = ensureStyleContrast(result);

  return result;
}

// LLM-generated diagrams often add `style X fill:#f9f` with a light fill but no
// text color. The app renders Mermaid with the dark theme (light text), so a
// light fill leaves light-on-light, unreadable text (reported from the Code
// Assistant). For any style/classDef declaration that sets a fill but no color,
// append a text color chosen for contrast against that fill.
function ensureStyleContrast(code: string): string {
  return code.replace(
    /((?:^|\n)[ \t]*(?:style|classDef)\b[^\n]*?fill\s*:\s*#([0-9a-fA-F]{3,8})[^\n]*)/g,
    (line: string, _full: string, hex: string) => {
      if (/\bcolor\s*:/.test(line)) return line; // author already set a text color
      const text = relativeLuminance(hex) > 0.55 ? "#111827" : "#f3f4f6";
      return `${line},color:${text}`;
    },
  );
}

// Perceptual luminance (0–1) of a #RGB / #RRGGBB / #RRGGBBAA hex color.
function relativeLuminance(hex: string): number {
  let h = hex.slice(0, 6);
  if (hex.length === 3) h = hex.split("").map((c) => c + c).join("");
  if (h.length < 6) h = h.padEnd(6, h[h.length - 1] ?? "0");
  const r = parseInt(h.slice(0, 2), 16);
  const g = parseInt(h.slice(2, 4), 16);
  const b = parseInt(h.slice(4, 6), 16);
  return (0.299 * r + 0.587 * g + 0.114 * b) / 255;
}

// Sentinel wrapping a masked-out quoted span. A private-use-area code point
// never appears in real Mermaid source, so it can't collide with content, and
// computing it at runtime keeps any special byte out of the source file.
const MASK = String.fromCharCode(0xE000);

// A masked span (a "..." that was already quoted) looks like <PUA>3<PUA>. The
// diamond/edge passes must leave these untouched — they already represent a
// quoted string, so re-wrapping them would double-quote and corrupt the label.
function isMaskedQuoted(inner: string): boolean {
  return new RegExp(`^${MASK}\\d+${MASK}$`).test(inner.trim());
}

function wrapQuoted(inner: string): string {
  // "→' (a straight " would re-open the label as a STR token) and <>→fullwidth
  // (angle brackets are lexed as HTML tags and break the parse).
  const safe = inner.replace(/"/g, "'").replace(/</g, "＜").replace(/>/g, "＞");
  return `"${safe}"`;
}

// Edge labels the model broke by embedding the delimiter `|`, quotes, or angle
// brackets in the label text. Anchored on the arrow (a run of link chars) and
// bounded by the `|` that precedes the target node, so it only rewrites genuine
// edge labels. Strips the stray delimiters/quotes and neutralizes <>, then
// re-wraps once. Assumes one edge per line (the usual LLM layout); a node label
// that itself contains a raw `|` could be over-matched, but that is far rarer
// than the broken-edge case this fixes.
function repairEdgeLabels(code: string): string {
  return code.replace(
    /([-.=<>ox]{2,})[ \t]*\|([^\n]*)\|(?=[ \t]*[A-Za-z0-9_])/g,
    (_full: string, link: string, label: string) => {
      const clean = label
        .replace(/"/g, "")
        .replace(/\|/g, " ")
        .replace(/</g, "＜")
        .replace(/>/g, "＞")
        .replace(/\s+/g, " ")
        .trim();
      return `${link}|"${clean}"|`;
    },
  );
}

// Shape syntaxes that reuse [...] but aren't plain rectangles — must not be
// rewrapped in quotes or their shape (cylinder/parallelogram/trapezoid)
// would be lost.
function isNonRectangleBracketShape(inner: string): boolean {
  return (
    /^\(.*\)$/.test(inner) ||   // [(cylinder)]
    /^\/.*\/$/.test(inner) ||   // [/parallelogram/]
    /^\\.*\\$/.test(inner) ||   // [\parallelogram\]
    /^\/.*\\$/.test(inner) ||   // [/trapezoid\]
    /^\\.*\/$/.test(inner)      // [\trapezoid/]
  );
}

// Rectangle node labels: A[text] → A["text"].
function quoteBracketLabels(code: string): string {
  // Negative lookbehind/lookahead skip the inner brackets of [[subroutine]].
  return code.replace(/(?<!\[)\[([^[\]\n]*)\](?!\])/g, (match, inner: string) => {
    if (!inner) return match;
    if (inner.startsWith('"') && inner.endsWith('"')) return match;
    if (isNonRectangleBracketShape(inner)) return match;
    return `[${wrapQuoted(inner)}]`;
  });
}

// Diamond decision node labels: C{text} → C{"text"}.
function quoteDiamondLabels(code: string): string {
  // Negative lookbehind/lookahead skip the inner braces of {{hexagon}}.
  return code.replace(/(?<!\{)\{([^{}\n]*)\}(?!\})/g, (match, inner: string) => {
    if (!inner) return match;
    if (isMaskedQuoted(inner)) return match;
    if (inner.startsWith('"') && inner.endsWith('"')) return match;
    return `{${wrapQuoted(inner)}}`;
  });
}

// Edge labels: -->|text|--> → -->|"text"|-->.
function quoteEdgeLabels(code: string): string {
  return code.replace(/\|([^\n|]*)\|/g, (match, inner: string) => {
    if (!inner) return match;
    if (isMaskedQuoted(inner)) return match;
    if (inner.startsWith('"') && inner.endsWith('"')) return match;
    return `|${wrapQuoted(inner)}|`;
  });
}

// Pass 2 — aggressive fallback when Pass 1 still fails to parse.
// Removes double-quote characters inside [...] and (...) node labels,
// which are the most common source of STR token errors.
export function aggressiveSanitize(code: string): string {
  return code
    // Strip " inside square-bracket node labels
    .replace(/\[([^\]]*)\]/g, (_m, inner: string) => "[" + inner.replace(/"/g, "'") + "]")
    // Strip " inside round-bracket node labels
    .replace(/\(([^)]*)\)/g, (_m, inner: string) => "(" + inner.replace(/"/g, "'") + ")")
    // Strip " inside subgraph labels (subgraph ID ["label"])
    .replace(/(subgraph\s+\w*\s*)\[([^\]]*)\]/g, (_m, pre: string, label: string) =>
      pre + "[" + label.replace(/"/g, "'") + "]"
    );
}
