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
  projectId,
  card,
  onClose,
  onSaved,
}: {
  projectId: string;
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
  const [interactive, setInteractive] = useState(card?.interactive ?? false);
  // Edit mode: already-uploaded rows, hanging off the existing card id.
  const [attachments, setAttachments] = useState<AttachmentRow[]>(card?.attachments ?? []);
  // Create mode: a brand-new card has no id yet, so picked files can't be
  // uploaded until `createTask` returns one — buffered here and uploaded in
  // `save()` right after that id comes back.
  const [pendingFiles, setPendingFiles] = useState<File[]>([]);
  const [busy, setBusy] = useState(false);

  const pickDir = async () => {
    const picked = await open({ directory: true, defaultPath: dir || undefined });
    if (typeof picked === "string") {
      setDir(picked);
    }
  };

  const onFiles = async (files: FileList | null) => {
    if (!files) return;
    if (isEdit) {
      for (const f of Array.from(files)) {
        const bytes = new Uint8Array(await f.arrayBuffer());
        const row = await addAttachment(projectId, card.id, f.name, bytes);
        setAttachments((a) => [...a, row]);
      }
    } else {
      setPendingFiles((prev) => [...prev, ...Array.from(files)]);
    }
  };

  const save = async () => {
    setBusy(true);
    try {
      if (dir) localStorage.setItem(LAST_DIR_KEY, dir);
      if (isEdit) {
        await updateTask(projectId, {
          id: card.id,
          title,
          body,
          project_dir: dir,
          parallel_ok: parallelOk,
          interactive,
        });
      } else {
        const newId = await createTask(projectId, {
          title,
          body,
          project_dir: dir,
          parallel_ok: parallelOk,
          interactive,
        });
        for (const f of pendingFiles) {
          const bytes = new Uint8Array(await f.arrayBuffer());
          await addAttachment(projectId, newId, f.name, bytes);
        }
      }
      onSaved();
    } finally {
      setBusy(false);
    }
  };

  const attachmentNames = isEdit
    ? attachments.map((a) => ({ key: a.id, name: a.filename }))
    : pendingFiles.map((f, i) => ({ key: `${i}-${f.name}`, name: f.name }));

  const removeAttachmentAt = (key: string) => {
    if (isEdit) {
      void removeAttachment(projectId, key).then(() =>
        setAttachments((list) => list.filter((x) => x.id !== key)),
      );
    } else {
      setPendingFiles((prev) => prev.filter((f, i) => `${i}-${f.name}` !== key));
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
            checked={interactive}
            onChange={(e) => setInteractive(e.target.checked)}
          />
          <span>{t.board_card_interactive}</span>
        </label>
        <p className="task-field-hint">{t.board_card_interactive_hint}</p>

        {!interactive && (
          <>
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
          </>
        )}

        <div className="task-field">
          <span className="task-field-label">{t.board_card_attachments}</span>
          <p className="task-field-hint">{t.board_card_attachments_hint}</p>
          {attachmentNames.length > 0 && (
            <ul className="task-attachment-list">
              {attachmentNames.map((a) => (
                <li key={a.key} className="task-attachment-row">
                  <span className="task-attachment-name">{a.name}</span>
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                    aria-label={`${t.board_delete} ${a.name}`}
                    onClick={() => removeAttachmentAt(a.key)}
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
