import { useEffect, useRef } from "react";
import { useLocale } from '../contexts/LocaleContext';
import "./StreamingIndicator.css";

interface StreamingIndicatorProps {
  text: string;
  visible: boolean;
  /** 按下停止鈕或 Esc。 */
  onStop?: () => void;
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

export function StreamingIndicator({ text, visible, onStop }: StreamingIndicatorProps) {
  const { t } = useLocale();
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [text]);

  // 這個狀態下輸入框被換掉了，沒有東西會吃到 Esc，所以可以放心全域監聽。
  useEffect(() => {
    if (!visible || !onStop) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onStop();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [visible, onStop]);

  if (!visible) return null;

  const explanation = extractPartialExplanation(text);

  return (
    <div className="aiterm-streaming">
      <div className="aiterm-streaming__header">
        <div className="aiterm-streaming__label">{t.streaming_generating}</div>
        {onStop && (
          <button type="button" className="aiterm-streaming__stop" onClick={onStop}>
            ■ {t.streaming_stop} <kbd>Esc</kbd>
          </button>
        )}
      </div>
      <div ref={scrollRef} className="aiterm-streaming__text">
        {explanation ?? t.streaming_thinking}
        <span className="aiterm-streaming__cursor" />
      </div>
    </div>
  );
}
