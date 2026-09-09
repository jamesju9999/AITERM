import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { usedLabels } from "../../ipc/projects";
import { setTaskLabel } from "../../ipc/tasks";

/**
 * queued/running/done 卡片專用的 Label 快速編輯視窗——只有一個 Label
 * 欄位，不牽涉 title/body/project_dir，存檔走 `setTaskLabel`
 * （`tasks_set_label`），不受「只有 planning 能編輯」的 `edit_allowed`
 * 限制。`planning` 卡片走的是完整的 `TaskEditorDialog`，不用這個。
 */
export function TaskLabelDialog({
  projectId,
  taskId,
  label: initialLabel,
  onClose,
  onSaved,
}: {
  projectId: string;
  taskId: string;
  label: string | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useLocale();
  const [label, setLabel] = useState(initialLabel ?? "");
  const [choices, setChoices] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    void usedLabels(projectId).then((labels) => {
      if (alive) setChoices(labels);
    });
    return () => {
      alive = false;
    };
  }, [projectId]);

  const save = async () => {
    setBusy(true);
    try {
      await setTaskLabel(projectId, taskId, label.trim() || null);
      onSaved();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div className="task-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="task-dialog-title">{t.board_action_edit_label}</h3>

        <label className="task-field">
          <span className="task-field-label">{t.board_card_label}</span>
          <input
            className="task-field-input"
            data-testid="task-label-quick-input"
            placeholder={t.board_card_label_placeholder}
            value={label}
            onChange={(e) => setLabel(e.target.value)}
          />
          {choices.length > 0 && (
            <div className="task-used-dirs" data-testid="used-labels-quick-row">
              {choices.map((l) => (
                <button
                  key={l}
                  type="button"
                  className="tb-btn tb-btn--ghost tb-btn--tiny"
                  data-testid={`used-label-quick-${l}`}
                  onClick={() => setLabel(l)}
                >
                  🏷 {l}
                </button>
              ))}
            </div>
          )}
        </label>

        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" disabled={busy} onClick={onClose}>
            {t.board_cancel}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary"
            disabled={busy}
            onClick={() => void save()}
          >
            {t.board_save}
          </button>
        </div>
      </div>
    </div>
  );
}
