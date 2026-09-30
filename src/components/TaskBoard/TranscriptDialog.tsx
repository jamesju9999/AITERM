import { useEffect, useMemo, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { readTranscript } from "../../ipc/tasks";
import { collapseConsecutiveDuplicateLines, parseTranscriptTurns } from "./transcriptUtils";

// 只載入最後這麼多字元；整份 70MB 的 session 丟進 DOM 會凍結視窗。
const TAIL_CHARS = 300_000;

export function TranscriptDialog({
  projectId,
  taskId,
  body,
  onClose,
}: {
  projectId: string;
  taskId: string;
  body: string;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const [text, setText] = useState<string | null>(null);
  const [maximized, setMaximized] = useState(false);
  const [showRaw, setShowRaw] = useState(false);
  // Keys are turn indexes; -1 is the preamble.
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set());
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
    void readTranscript(projectId, taskId, TAIL_CHARS).then((s) => {
      if (alive) setText(s);
    });
    return () => {
      alive = false;
    };
  }, [projectId, taskId]);

  const raw = text === null ? null : collapseConsecutiveDuplicateLines(text);
  const parsed = useMemo(() => (raw === null ? null : parseTranscriptTurns(raw)), [raw]);
  const hasTurns = !!parsed && parsed.turns.length > 0;
  const foldable = hasTurns && !showRaw;

  const toggleTurn = (key: number) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  const expandAll = () => {
    if (!parsed) return;
    setExpanded(new Set([-1, ...parsed.turns.keys()]));
  };

  const renderTurn = (key: number, title: string, body: string) => {
    const open = expanded.has(key);
    return (
      <div className="task-transcript-turn" key={key}>
        <button
          className="task-transcript-turn-head"
          aria-expanded={open}
          onClick={() => toggleTurn(key)}
        >
          <span aria-hidden="true">{open ? "▾" : "▸"}</span> {title}
        </button>
        {open && <pre className="task-transcript-raw task-transcript-turn-body">{body}</pre>}
      </div>
    );
  };

  return (
    <div className="task-dialog-backdrop">
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
          <div className="task-transcript-head">
            <span className="task-field-label">{t.board_transcript_raw_label}</span>
            {hasTurns && (
              <span className="task-transcript-controls">
                {foldable && (
                  <>
                    <button className="tb-btn tb-btn--ghost" onClick={expandAll}>
                      {t.board_transcript_expand_all}
                    </button>
                    <button className="tb-btn tb-btn--ghost" onClick={() => setExpanded(new Set())}>
                      {t.board_transcript_collapse_all}
                    </button>
                  </>
                )}
                <button className="tb-btn tb-btn--ghost" onClick={() => setShowRaw((r) => !r)}>
                  {showRaw ? t.board_transcript_show_turns : t.board_transcript_show_raw}
                </button>
              </span>
            )}
          </div>
          {foldable && parsed ? (
            <div className="task-transcript-turns" data-testid="task-transcript-turns">
              {parsed.preamble && renderTurn(-1, t.board_transcript_preamble, parsed.preamble)}
              {parsed.turns.map((turn, i) =>
                renderTurn(i, `#${i + 1} ${turn.prompt}`, turn.output || t.board_transcript_empty),
              )}
            </div>
          ) : (
            <pre className="task-transcript-raw" data-testid="task-transcript-raw">
              {raw === null ? "…" : raw || t.board_transcript_empty}
            </pre>
          )}
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
