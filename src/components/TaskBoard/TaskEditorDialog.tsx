import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import {
  addAttachment,
  createTask,
  removeAttachment,
  updateTask,
  type AttachmentRow,
  type TaskWithAttachments,
} from "../../ipc/tasks";

const LAST_DIR_KEY = "aiterm_last_task_dir";

export function TaskEditorDialog({
  card,
  onClose,
  onSaved,
}: {
  card: TaskWithAttachments | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useLocale();
  const isEdit = card !== null;
  const [title, setTitle] = useState(card?.title ?? "");
  const [body, setBody] = useState(card?.body ?? "");
  const [dir, setDir] = useState(card?.project_dir ?? localStorage.getItem(LAST_DIR_KEY) ?? "");
  const [parallelOk, setParallelOk] = useState(card?.parallel_ok ?? true);
  const [attachments, setAttachments] = useState<AttachmentRow[]>(card?.attachments ?? []);
  const [busy, setBusy] = useState(false);

  const pickDir = async () => {
    const picked = await open({ directory: true, defaultPath: dir || undefined });
    if (typeof picked === "string") {
      setDir(picked);
    }
  };

  const onFiles = async (files: FileList | null) => {
    if (!files || !card) return;
    for (const f of Array.from(files)) {
      const bytes = new Uint8Array(await f.arrayBuffer());
      const row = await addAttachment(card.id, f.name, bytes);
      setAttachments((a) => [...a, row]);
    }
  };

  const save = async () => {
    setBusy(true);
    try {
      if (dir) localStorage.setItem(LAST_DIR_KEY, dir);
      if (isEdit) {
        await updateTask({ id: card.id, title, body, project_dir: dir, parallel_ok: parallelOk });
      } else {
        await createTask({ title, body, project_dir: dir, parallel_ok: parallelOk });
      }
      onSaved();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div className="task-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="task-dialog-title">{isEdit ? t.board_edit_card : t.board_new_card}</h3>

        <label className="task-field">
          <span className="task-field-label">{t.board_card_title}</span>
          <input className="task-field-input" value={title} onChange={(e) => setTitle(e.target.value)} />
        </label>

        <label className="task-field">
          <span className="task-field-label">{t.board_card_body}</span>
          <textarea
            className="task-field-input task-field-textarea"
            rows={5}
            value={body}
            onChange={(e) => setBody(e.target.value)}
          />
        </label>

        <label className="task-field">
          <span className="task-field-label">{t.board_card_folder}</span>
          <div className="task-field-row">
            <input className="task-field-input" value={dir} onChange={(e) => setDir(e.target.value)} />
            <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void pickDir()}>
              {t.board_card_folder_pick}
            </button>
          </div>
        </label>

        <label className="task-field task-field--checkbox">
          <input
            type="checkbox"
            className="task-checkbox"
            checked={parallelOk}
            onChange={(e) => setParallelOk(e.target.checked)}
          />
          <span>{t.board_card_parallel}</span>
        </label>
        <p className="task-field-hint">{t.board_card_solo_hint}</p>

        {isEdit && (
          <div className="task-field">
            <span className="task-field-label">{t.board_card_attachments}</span>
            <p className="task-field-hint">{t.board_card_attachments_hint}</p>
            {attachments.length > 0 && (
              <ul className="task-attachment-list">
                {attachments.map((a) => (
                  <li key={a.id} className="task-attachment-row">
                    <span className="task-attachment-name">{a.filename}</span>
                    <button
                      type="button"
                      className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                      aria-label={`${t.board_delete} ${a.filename}`}
                      onClick={() =>
                        void removeAttachment(a.id).then(() =>
                          setAttachments((list) => list.filter((x) => x.id !== a.id)),
                        )
                      }
                    >
                      ✕
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <label className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm task-attachment-add">
              + {t.board_add_attachment}
              <input
                type="file"
                multiple
                className="task-attachment-file-input"
                aria-label={t.board_add_attachment}
                onChange={(e) => void onFiles(e.target.files)}
              />
            </label>
          </div>
        )}

        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" disabled={busy} onClick={onClose}>
            {t.board_cancel}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary"
            disabled={busy || !title.trim() || !dir.trim()}
            onClick={() => void save()}
          >
            {t.board_save}
          </button>
        </div>
      </div>
    </div>
  );
}
