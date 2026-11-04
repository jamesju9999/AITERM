import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { addProjectNote, deleteProjectNote, listProjectNotes, type ProjectNote } from "../../ipc/tasks";

/** 專案記事本：Agent 用 remember 留下、之後每張卡都會帶的共用筆記。改動立即生效（不跟簡介一起按儲存）。 */
export function ProjectNotes({ projectId }: { projectId: string }) {
  const { t } = useLocale();
  const [notes, setNotes] = useState<ProjectNote[] | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const list = await listProjectNotes(projectId);
        if (alive) setNotes(list);
      } catch (e) {
        if (!alive) return;
        setNotes([]);
        setError(String(e));
      }
    })();
    return () => {
      alive = false;
    };
  }, [projectId]);

  const add = async () => {
    const text = draft.trim();
    if (!text) return;
    setBusy(true);
    setError(null);
    try {
      const saved = await addProjectNote(projectId, text);
      // 後端會擋掉字句完全相同的，回傳既有那則；別讓清單出現兩筆。
      setNotes((cur) => [...(cur ?? []).filter((n) => n.id !== saved.id), saved]);
      setDraft("");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (id: number) => {
    setError(null);
    try {
      await deleteProjectNote(projectId, id);
      setNotes((cur) => (cur ?? []).filter((n) => n.id !== id));
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <section className="ac-notes" aria-label={t.ac_notes_title}>
      <h4 className="ac-notes-title">{t.ac_notes_title}</h4>
      <p className="ac-dialog-desc">{t.ac_notes_desc}</p>
      <p className="task-field-hint" data-testid="notes-limit">
        {t.ac_notes_limit}
      </p>
      {notes !== null && notes.length === 0 && (
        <p className="task-field-hint" data-testid="notes-empty">
          {t.ac_notes_empty}
        </p>
      )}
      <ul className="ac-notes-list">
        {(notes ?? []).map((n) => (
          <li key={n.id} className="ac-note">
            <span className="ac-note-author">{n.author || t.ac_notes_user}</span>
            <span className="ac-note-text">{n.text}</span>
            <button
              type="button"
              className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
              data-testid={`note-delete-${n.id}`}
              aria-label={t.ac_notes_delete}
              title={t.ac_notes_delete}
              onClick={() => void remove(n.id)}
            >
              ×
            </button>
          </li>
        ))}
      </ul>
      <div className="ac-note-add">
        <input
          className="task-field-input"
          data-testid="note-input"
          value={draft}
          placeholder={t.ac_notes_placeholder}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.nativeEvent.isComposing && e.keyCode !== 229) {
              e.preventDefault();
              void add();
            }
          }}
        />
        <button
          type="button"
          className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
          data-testid="note-add"
          disabled={busy || !draft.trim()}
          onClick={() => void add()}
        >
          {t.ac_notes_add}
        </button>
      </div>
      {error && <p className="ac-bad-text">{error}</p>}
    </section>
  );
}
