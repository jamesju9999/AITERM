import { useEffect } from "react";
import "./CommandPreview.css";

export interface CommandPreviewProps {
  command: string;
  explanation: string;
  onConfirm: () => void;
  onCancel: () => void;
}

export function CommandPreview({
  command,
  explanation,
  onConfirm,
  onCancel,
}: CommandPreviewProps) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        onConfirm();
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        onCancel();
      }
    };
    // Capture phase so we intercept before xterm.js sees the key.
    document.addEventListener("keydown", handler, true);
    return () => document.removeEventListener("keydown", handler, true);
  }, [onConfirm, onCancel]);

  return (
    <div className="aiterm-command-preview" role="dialog" aria-label="AI command preview">
      <div className="aiterm-command-preview__label">Command</div>
      <div className="aiterm-command-preview__command">{command}</div>
      <div className="aiterm-command-preview__label">Explanation</div>
      <div className="aiterm-command-preview__explanation">{explanation}</div>
      <div className="aiterm-command-preview__hint">
        [Enter] Execute &nbsp;&nbsp; [Esc] Cancel
      </div>
    </div>
  );
}
