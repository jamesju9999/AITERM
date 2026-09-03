import { useEffect, useRef, useState } from "react";
import mermaid from "mermaid";
import { useLocale } from '../contexts/LocaleContext';
import { sanitizeMermaid, aggressiveSanitize } from "../lib/mermaidSanitize";
import { MERMAID_INIT } from "../lib/mermaidTheme";

mermaid.initialize(MERMAID_INIT);

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
