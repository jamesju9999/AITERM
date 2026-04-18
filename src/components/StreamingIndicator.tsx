import { useEffect, useRef } from "react";
import "./StreamingIndicator.css";

interface StreamingIndicatorProps {
  text: string;
  visible: boolean;
}

export function StreamingIndicator({ text, visible }: StreamingIndicatorProps) {
  const scrollRef = useRef<HTMLPreElement>(null);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [text]);

  if (!visible) return null;

  return (
    <div className="aiterm-streaming">
      <div className="aiterm-streaming__label">AI generating…</div>
      <pre ref={scrollRef} className="aiterm-streaming__text">
        {text}
        <span className="aiterm-streaming__cursor" />
      </pre>
    </div>
  );
}
