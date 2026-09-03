import { useEffect, useState } from "react";

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
      <div className="task-dialog task-transcript-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="task-dialog-title">{t.board_transcript_title}</h3>

        <div className="task-field">
          <span className="task-field-label">{t.board_transcript_prompt_label}</span>
          <p className="task-transcript-prompt">{body}</p>
        </div>

        <div className="task-field">
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
