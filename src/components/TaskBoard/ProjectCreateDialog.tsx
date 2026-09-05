import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { createProject } from "../../ipc/projects";

/** 沿用 TaskEditorDialog 記住上次目錄的做法，但用自己的 key。 */
const LAST_PARENT_KEY = "aiterm_last_project_parent";

export function ProjectCreateDialog({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (projectId: string) => void;
}) {
  const { t } = useLocale();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [parent, setParent] = useState(localStorage.getItem(LAST_PARENT_KEY) ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pickParent = async () => {
    const picked = await open({ directory: true, defaultPath: parent || undefined });
    if (typeof picked === "string") setParent(picked);
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      localStorage.setItem(LAST_PARENT_KEY, parent);
      const id = await createProject({ parentDir: parent, name: name.trim(), description });
      onCreated(id);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const ready = name.trim().length > 0 && parent.length > 0 && !busy;

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div className="task-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="task-dialog-title">{t.proj_new}</h3>

        <label className="task-field">
          <span className="task-field-label">{t.proj_create_name}</span>
          <input
            className="task-field-input"
            data-testid="project-create-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </label>

        <label className="task-field">
          <span className="task-field-label">{t.proj_create_desc}</span>
          <input
            className="task-field-input"
            data-testid="project-create-desc"
            value={description}
            onChange={(e) => setDescription(e.target.value)}
          />
        </label>

        <label className="task-field">
          <span className="task-field-label">{t.proj_create_parent}</span>
          <div className="task-field-row">
            <input
              className="task-field-input"
              readOnly
              value={parent}
              data-testid="project-create-parent"
            />
            <button
              type="button"
              className="tb-btn tb-btn--ghost"
              data-testid="project-create-browse"
              onClick={() => void pickParent()}
            >
              {t.proj_create_browse}
            </button>
          </div>
        </label>

        {parent && name.trim() && (
          <div className="task-field-hint">
            {t.proj_create_preview}：{parent}/{name.trim()}
          </div>
        )}
        {error && <div className="task-field-hint">{error}</div>}

        <div className="task-dialog-actions">
          <button className="tb-btn tb-btn--ghost" disabled={busy} onClick={onClose}>
            {t.proj_create_cancel}
          </button>
          <button
            className="tb-btn tb-btn--primary"
            data-testid="project-create-submit"
            disabled={!ready}
            onClick={() => void submit()}
          >
            {t.proj_create_submit}
          </button>
        </div>
      </div>
    </div>
  );
}
