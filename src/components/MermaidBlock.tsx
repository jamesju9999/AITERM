import { useEffect, useRef, useState } from "react";
import mermaid from "mermaid";
import { useLocale } from '../contexts/LocaleContext';

mermaid.initialize({
  startOnLoad: false,
  theme: "dark",
  securityLevel: "loose", // needed for some interactive diagram features or styling
});

// Sanitize Mermaid code to fix common model-generated syntax issues:
// 1. Fullwidth/Halfwidth Forms (U+FF01–U+FF60) → ASCII equivalents
// 2. <br/> / <br> tags in node labels → space  (<br/>'s ">" confuses the lexer
//    into reading it as a LINK_ID token, causing "got 'LINK_ID'" parse errors)
// 3. Edge labels containing parentheses → quoted so "(" isn't read as a node shape
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
    .replace(/「|」|『|』|"|"/g, '"')
    .replace(/『|』/g, "'");

  // Replace <br/>, <br />, <br> with a space.
  // The ">" in <br/> is tokenised as a LINK_ID (arrow) by Mermaid's lexer
  // when it appears inside a node label like [Text<br/>More].
  result = result.replace(/<br\s*\/?>/gi, " ");

  // Quote edge labels that contain parentheses.
  // Matches |label| where label has no existing quotes and contains ( or ).
  result = result.replace(/\|([^|"]*[()][^|"]*)\|/g, '|"$1"|');

  return result;
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

    async function renderMermaid() {
      const trimmed = sanitizeMermaid(chart.trim());
      if (!trimmed) return;

      try {
        // Validate first — prevents mermaid from leaking its bomb error UI into document.body
        await mermaid.parse(trimmed);
      } catch (err: any) {
        if (!isCancelled) {
          setError(err.message || "Mermaid parse error");
        }
        return;
      }

      try {
        const id = `mermaid-${Math.random().toString(36).substring(2, 9)}`;
        const { svg } = await mermaid.render(id, trimmed);
        if (!isCancelled) {
          setSvgStr(svg);
          setError(null);
        }
      } catch (err: any) {
        if (!isCancelled) {
          setError(err.message || "Failed to render Mermaid diagram");
        }
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
