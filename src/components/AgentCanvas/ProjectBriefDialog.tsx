import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { getProjectBrief, setProjectBrief } from "../../ipc/tasks";

/** 超過就提醒：每張卡都會帶著這段（後端放進提示時另有更寬的硬上限）。 */
const BRIEF_SOFT_LIMIT = 2000;

/** 專案簡介：每張 Agent 卡派工時放在提示前面，連同自動產生的團隊分工。 */
export function ProjectBriefDialog({
  projectId,
  onSaved,
  onClose,
}: {
  projectId: string;
  onSaved: (brief: string) => void;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const [text, setText] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    getProjectBrief(projectId)
      .then((b) => alive && setText(b ?? ""))
      .catch((e) => {
        if (!alive) return;
        setText("");
        setError(String(e));
      });
    return () => {
      alive = false;
    };
  }, [projectId]);

  const save = async () => {
    if (text === null) return;
    setBusy(true);
    setError(null);
    try {
      await setProjectBrief(projectId, text.trim() || null);
      onSaved(text.trim());
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const count = [...(text ?? "")].length;
  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-brief" role="dialog" aria-label={t.ac_brief_title}>
        <h3 className="task-dialog-title">{t.ac_brief_title}</h3>
        <p className="ac-dialog-desc">{t.ac_brief_desc}</p>
        <textarea
          className="task-field-input ac-brief-text"
          aria-label={t.ac_brief_title}
          value={text ?? ""}
          disabled={text === null}
          placeholder={t.ac_brief_placeholder}
          onChange={(e) => setText(e.target.value)}
        />
        <div className={`ac-brief-count${count > BRIEF_SOFT_LIMIT ? " ac-bad-text" : ""}`}>{t.ac_brief_count(count)}</div>
        {count > BRIEF_SOFT_LIMIT && <p className="task-field-hint ac-bad-text">{t.ac_brief_long}</p>}
        {error && <p className="ac-bad-text">{error}</p>}
        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.ac_brief_cancel}
          </button>
          <button className="aiterm-btn aiterm-btn--primary" disabled={busy || text === null} onClick={() => void save()}>
            {t.ac_brief_save}
          </button>
        </div>
      </div>
    </div>
  );
}
