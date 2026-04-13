import { useEffect } from "react";
import type { RiskLevel } from "../ipc/ai";
import "./CommandPreview.css";

export interface CommandPreviewProps {
  command: string;
  explanation: string;
  riskLevel: RiskLevel;
  onConfirm: () => void;
  onCancel: () => void;
}

const RISK_LABELS: Record<RiskLevel, string> = {
  safe: "Safe",
  needs_confirm: "Caution",
  dangerous: "Dangerous",
};

export function CommandPreview({
  command,
  explanation,
  riskLevel,
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
      <div className="aiterm-command-preview__header">
        <span className="aiterm-command-preview__label">Command</span>
        <span className={`aiterm-command-preview__risk aiterm-command-preview__risk--${riskLevel}`}>
          {RISK_LABELS[riskLevel]}
        </span>
      </div>
      <div className="aiterm-command-preview__command">{command}</div>
      <div className="aiterm-command-preview__label">Explanation</div>
      <div className="aiterm-command-preview__explanation">{explanation}</div>
      <div className="aiterm-command-preview__actions">
        <button
          className={`aiterm-command-preview__confirm aiterm-command-preview__confirm--${riskLevel}`}
          onClick={onConfirm}
        >
          {riskLevel === "dangerous" ? "Execute Anyway" : "Execute"}
        </button>
        <span className="aiterm-command-preview__hint">
          [Enter] Execute &nbsp;&nbsp; [Esc] Cancel
        </span>
      </div>
    </div>
  );
}
