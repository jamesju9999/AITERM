import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { readTranscript } from "../../ipc/tasks";

export function TranscriptDialog({ taskId, onClose }: { taskId: string; onClose: () => void }) {
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

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div className="task-dialog" onClick={(e) => e.stopPropagation()}>
        <h3>{t.board_transcript_title}</h3>
        <pre style={{ whiteSpace: "pre-wrap", maxHeight: "60vh", overflowY: "auto", margin: 0 }}>
          {text === null ? "…" : text || t.board_transcript_empty}
        </pre>
        <button onClick={onClose}>{t.board_cancel}</button>
      </div>
    </div>
  );
}
