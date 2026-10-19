import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { Reference } from "../../ipc/agents";
import { RoleIcon } from "./RoleIcon";

/** 專案的參考資料夾：整個團隊都能讀、不能寫（例如舊系統原始碼與它的文件）。 */
export function ReferencesDialog({
  initial,
  onSave,
  onClose,
}: {
  initial: Reference[];
  onSave: (refs: Reference[]) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const [refs, setRefs] = useState<Reference[]>(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const add = async () => {
    try {
      const picked = await open({ directory: true });
      if (typeof picked !== "string" || !picked) return;
      setRefs((r) => (r.some((x) => x.path === picked) ? r : [...r, { path: picked, note: "" }]));
    } catch (e) {
      setError(String(e));
    }
  };

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await onSave(refs);
      onClose();
    } catch (e) {
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    } finally {
      setBusy(false);
    }
  };

  const folderName = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-refs-dialog" role="dialog" aria-label={t.ac_refs_title} data-testid="references-dialog">
        <h3 className="task-dialog-title ac-inline-icon">
          <span className="ac-agent-ico ac-skill-ico-db" aria-hidden>
            <RoleIcon name="folder" />
          </span>
          {t.ac_refs_title}
        </h3>
        <p className="ac-dialog-desc">{t.ac_refs_desc}</p>
        {refs.length === 0 && <p className="task-field-hint">{t.ac_refs_empty}</p>}
        <div className="ac-refs-list">
          {refs.map((r) => (
            <div key={r.path} className="ac-refs-item">
              <div className="ac-refs-head">
                <RoleIcon name="folder" size={14} />
                <span className="ac-refs-name">{folderName(r.path)}</span>
                <span className="ac-refs-path" title={r.path}>
                  {r.path}
                </span>
                <button
                  type="button"
                  className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                  onClick={() => setRefs((all) => all.filter((x) => x.path !== r.path))}
                >
                  {t.ac_refs_remove}
                </button>
              </div>
              <input
                className="task-field-input"
                aria-label={`${t.ac_refs_note} ${r.path}`}
                placeholder={t.ac_refs_note_placeholder}
                value={r.note}
                onChange={(e) => setRefs((all) => all.map((x) => (x.path === r.path ? { ...x, note: e.target.value } : x)))}
              />
            </div>
          ))}
        </div>
        <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm ac-refs-add" onClick={() => void add()}>
          ＋ {t.ac_refs_add}
        </button>
        {error && <p className="ac-bad-text">{error}</p>}
        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.cancel}
          </button>
          <button className="aiterm-btn aiterm-btn--primary" disabled={busy} onClick={() => void save()}>
            {t.ac_db_save}
          </button>
        </div>
      </div>
    </div>
  );
}
