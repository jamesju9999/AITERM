import { useEffect, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { readTranscript } from "../../ipc/tasks";
import { collapseConsecutiveDuplicateLines } from "./transcriptUtils";

export function TranscriptDialog({
  taskId,
  body,
  onClose,
}: {
  taskId: string;
  body: string;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const [text, setText] = useState<string | null>(null);
  const [maximized, setMaximized] = useState(false);
  const dialogRef = useRef<HTMLDivElement>(null);
  // CSS `resize` writes the dragged size as INLINE width/height on the
  // element, and inline styles beat the maximized class's own sizing — so
  // maximizing has to clear them, and restoring has to put them back.
  const draggedSize = useRef<{ width: string; height: string } | null>(null);

  const toggleMaximized = () => {
    const el = dialogRef.current;
    if (!el) return;
    if (maximized) {
      el.style.width = draggedSize.current?.width ?? "";
      el.style.height = draggedSize.current?.height ?? "";
    } else {
      draggedSize.current = { width: el.style.width, height: el.style.height };
      el.style.width = "";
      el.style.height = "";
    }
    setMaximized((m) => !m);
  };

  useEffect(() => {
    let alive = true;
    void readTranscript(taskId).then((s) => {
      if (alive) setText(s);
    });
    return () => {
      alive = false;
    };
  }, [taskId]);

  const raw = text === null ? null : collapseConsecutiveDuplicateLines(text);

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div
        ref={dialogRef}
        className={`task-dialog task-transcript-dialog${maximized ? " task-transcript-dialog--max" : ""}`}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="task-transcript-head">
          <h3 className="task-dialog-title">{t.board_transcript_title}</h3>
          <button
            className="tb-btn tb-btn--ghost"
            onClick={toggleMaximized}
            title={maximized ? t.board_transcript_restore : t.board_transcript_maximize}
            aria-label={maximized ? t.board_transcript_restore : t.board_transcript_maximize}
          >
            {maximized ? "⤡" : "⤢"}
          </button>
        </div>

        <div className="task-field">
          <span className="task-field-label">{t.board_transcript_prompt_label}</span>
          <p className="task-transcript-prompt">{body}</p>
        </div>

        <div className="task-field task-field--grow">
          <span className="task-field-label">{t.board_transcript_raw_label}</span>
          <pre className="task-transcript-raw" data-testid="task-transcript-raw">
            {raw === null ? "…" : raw || t.board_transcript_empty}
          </pre>
        </div>

        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.board_cancel}
          </button>
        </div>
      </div>
    </div>
  );
}
