import { useEffect, useRef } from "react";
import "./StreamingIndicator.css";

interface StreamingIndicatorProps {
  text: string;
  visible: boolean;
}

/**
 * The /ai flow asks the model to output JSON: {"explanation":"...","command":"...","risk_level":"..."}.
 * While streaming, raw JSON tokens are unreadable. Try to extract the "explanation" field
 * as it builds up so we show meaningful text instead of JSON syntax.
 */
function extractPartialExplanation(raw: string): string | null {
  // Match "explanation": "partial text (may be incomplete)
  const m = raw.match(/"explanation"\s*:\s*"((?:[^"\\]|\\.)*)"/);
  if (m) {
    return m[1].replace(/\\n/g, " ").replace(/\\"/g, '"').trim();
  }
  // Partial: field opened but string not yet closed
  const partial = raw.match(/"explanation"\s*:\s*"((?:[^"\\]|\\.)*)/);
  if (partial && partial[1].length > 0) {
    return partial[1].replace(/\\n/g, " ").replace(/\\"/g, '"').trim() + "…";
  }
  return null;
}

export function StreamingIndicator({ text, visible }: StreamingIndicatorProps) {
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [text]);

  if (!visible) return null;

  const explanation = extractPartialExplanation(text);

  return (
    <div className="aiterm-streaming">
      <div className="aiterm-streaming__label">AI 生成指令中…</div>
      <div ref={scrollRef} className="aiterm-streaming__text">
        {explanation ?? "思考中…"}
        <span className="aiterm-streaming__cursor" />
      </div>
    </div>
  );
}
