import React, { useEffect, useRef, useState } from "react";
import mermaid from "mermaid";

mermaid.initialize({
  startOnLoad: false,
  theme: "dark",
  securityLevel: "loose", // needed for some interactive diagram features or styling
});

interface MermaidBlockProps {
  chart: string;
}

export function MermaidBlock({ chart }: MermaidBlockProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [svgStr, setSvgStr] = useState<string>("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let isCancelled = false;

    async function renderMermaid() {
      const trimmed = chart.trim();
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

  if (error) {
    return (
      <div className="mermaid-error" style={{ color: "#f87171", padding: "10px", border: "1px solid #f87171", borderRadius: "4px", margin: "10px 0", fontSize: "12px" }}>
        <strong>Mermaid Error:</strong>
        <pre style={{ whiteSpace: "pre-wrap", marginTop: "5px", fontSize: "11px" }}>{error}</pre>
        <details style={{ marginTop: "5px", cursor: "pointer" }}>
          <summary>Source</summary>
          <pre style={{ fontSize: "11px", marginTop: "5px" }}>{chart}</pre>
        </details>
      </div>
    );
  }

  if (!svgStr) {
    return <div style={{ color: "#aaa", padding: "10px", textAlign: "center", fontSize: "12px" }}>Rendering diagram...</div>;
  }

  return (
    <div 
      ref={containerRef}
      className="mermaid-container"
      style={{ margin: "16px 0", display: "flex", justifyContent: "center", background: "rgba(255, 255, 255, 0.02)", padding: "12px", borderRadius: "8px" }}
      dangerouslySetInnerHTML={{ __html: svgStr }} 
    />
  );
}
