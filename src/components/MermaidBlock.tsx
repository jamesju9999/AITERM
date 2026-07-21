import { useEffect, useRef, useState } from "react";
import mermaid from "mermaid";
import { useLocale } from '../contexts/LocaleContext';

mermaid.initialize({
  startOnLoad: false,
  theme: "dark",
  securityLevel: "loose", // needed for some interactive diagram features or styling
});

// Pass 1 — conservative sanitization.
// Converts known problematic tokens, then unconditionally quotes node/edge
// labels in flowchart diagrams. Quoting a label is always syntactically
// safe in Mermaid, so wrapping every label removes the entire class of
// "unquoted special character confuses the lexer" errors (parens, angle
// brackets, colons, hashes, pipes, ...) instead of reacting to each one
// individually as it's discovered.
function sanitizeMermaid(code: string): string {
  let result = code
    // Full-width punctuation → ASCII
    .replace(/（/g, "(").replace(/）/g, ")")
    .replace(/【/g, "[").replace(/】/g, "]")
    .replace(/｛/g, "{").replace(/｝/g, "}")
    .replace(/＜/g, "<").replace(/＞/g, ">")
    .replace(/｜/g, "|")
    .replace(/，/g, ",").replace(/。/g, ".").replace(/、/g, ",")
    .replace(/：/g, ":").replace(/；/g, ";")
    // Chinese corner/curly quotes → ASCII SINGLE quotes.
    // Do NOT use " here: inside [node labels], Mermaid lexes "text" as a STR
    // token which triggers "got 'STR'" parse errors.
    .replace(/[「」『』""]/g, "'");

  // <br/> → space: the ">" is lexed as LINK_ID inside node labels.
  result = result.replace(/<br\s*\/?>/gi, " ");

  // Blanket label quoting only applies to flowchart/graph syntax — other
  // diagram types (classDiagram, stateDiagram, sequenceDiagram, ...) use
  // [] {} for unrelated constructs and would be corrupted by this pass.
  if (/^\s*(flowchart|graph)\b/im.test(result)) {
    result = quoteBracketLabels(result);
    result = quoteDiamondLabels(result);
    result = quoteEdgeLabels(result);
  }

  return result;
}

function wrapQuoted(inner: string): string {
  return `"${inner.replace(/"/g, "'")}"`;
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
    if (inner.startsWith('"') && inner.endsWith('"')) return match;
    return `{${wrapQuoted(inner)}}`;
  });
}

// Edge labels: -->|text|--> → -->|"text"|-->.
function quoteEdgeLabels(code: string): string {
  return code.replace(/\|([^\n|]*)\|/g, (match, inner: string) => {
    if (!inner) return match;
    if (inner.startsWith('"') && inner.endsWith('"')) return match;
    return `|${wrapQuoted(inner)}|`;
  });
}

// Pass 2 — aggressive fallback when Pass 1 still fails to parse.
// Removes double-quote characters inside [...] and (...) node labels,
// which are the most common source of STR token errors.
function aggressiveSanitize(code: string): string {
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

interface MermaidBlockProps {
  chart: string;
}

export function MermaidBlock({ chart }: MermaidBlockProps) {
  const { t } = useLocale();
  const containerRef = useRef<HTMLDivElement>(null);
  const [svgStr, setSvgStr] = useState<string>("");
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    let isCancelled = false;

    async function tryRender(source: string): Promise<boolean> {
      try {
        await mermaid.parse(source);
      } catch {
        return false;
      }
      try {
        const id = `mermaid-${Math.random().toString(36).substring(2, 9)}`;
        const { svg } = await mermaid.render(id, source);
        if (!isCancelled) { setSvgStr(svg); setError(null); }
        return true;
      } catch {
        return false;
      }
    }

    async function renderMermaid() {
      const raw = chart.trim();
      if (!raw) return;

      // Pass 1: conservative sanitization
      const pass1 = sanitizeMermaid(raw);
      if (await tryRender(pass1)) return;

      // Pass 2: aggressive — strip double quotes from node/subgraph labels
      const pass2 = aggressiveSanitize(pass1);
      if (await tryRender(pass2)) return;

      // Both passes failed — show error with the Pass 1 source for readability
      if (!isCancelled) {
        try { await mermaid.parse(pass1); }
        catch (err: any) { setError(err.message || "Mermaid parse error"); }
      }
    }

    if (chart) {
      renderMermaid();
    }

    return () => {
      isCancelled = true;
    };
  }, [chart]);

  // Close overlay on Escape key
  useEffect(() => {
    if (!expanded) return;
    const handler = (e: KeyboardEvent) => { if (e.key === "Escape") setExpanded(false); };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [expanded]);

  if (error) {
    return (
      <div className="mermaid-error" style={{ color: "#f87171", padding: "10px", border: "1px solid #f87171", borderRadius: "4px", margin: "10px 0", fontSize: "12px" }}>
        <strong>{t.mermaid_error}</strong>
        <pre style={{ whiteSpace: "pre-wrap", marginTop: "5px", fontSize: "11px" }}>{error}</pre>
        <details style={{ marginTop: "5px", cursor: "pointer" }}>
          <summary>{t.mermaid_source}</summary>
          <pre style={{ fontSize: "11px", marginTop: "5px" }}>{chart}</pre>
        </details>
      </div>
    );
  }

  if (!svgStr) {
    return <div style={{ color: "#aaa", padding: "10px", textAlign: "center", fontSize: "12px" }}>{t.mermaid_rendering}</div>;
  }

  return (
    <>
      {/* Thumbnail — click to expand */}
      <div
        ref={containerRef}
        className="mermaid-container"
        title={t.mermaid_click_to_expand}
        onClick={() => setExpanded(true)}
        dangerouslySetInnerHTML={{ __html: svgStr }}
      />

      {/* Full-screen overlay */}
      {expanded && (
        <div className="mermaid-overlay" onClick={() => setExpanded(false)}>
          <div className="mermaid-overlay__content" onClick={(e) => e.stopPropagation()}>
            <button
              className="mermaid-overlay__close"
              onClick={() => setExpanded(false)}
              title="關閉 (Esc)"
            >
              ✕
            </button>
            <div
              className="mermaid-overlay__diagram"
              dangerouslySetInnerHTML={{ __html: svgStr }}
            />
          </div>
        </div>
      )}
    </>
  );
}
