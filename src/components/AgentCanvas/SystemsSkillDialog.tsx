import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { systemsList, type SystemConnection } from "../../ipc/systems";
import { RoleIcon } from "./RoleIcon";

/** 系統連線技能的設定：勾選這位 Agent 能操作哪些系統。`current` 為空是新裝。 */
export function SystemsSkillDialog({
  agentName,
  current,
  onSave,
  onClose,
}: {
  agentName: string;
  current: string[];
  onSave: (ids: string[]) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const [systems, setSystems] = useState<SystemConnection[] | null>(null);
  const [picked, setPicked] = useState<string[]>(current);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    systemsList()
      .then((l) => alive && setSystems(l))
      .catch((e) => alive && setError(typeof e === "string" ? e : String(e)));
    return () => {
      alive = false;
    };
  }, []);

  const save = async (ids: string[]) => {
    setBusy(true);
    setError(null);
    try {
      await onSave(ids);
      onClose();
    } catch (e) {
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    } finally {
      setBusy(false);
    }
  };

  const modeLabel = (m: SystemConnection["mode"]) =>
    ({ read_only: t.sys_mode_title_read_only, approve_each: t.sys_mode_title_approve_each, rules: t.sys_mode_title_rules, autonomous: t.sys_mode_title_autonomous })[m];
  const missing = current.filter((id) => systems !== null && !systems.some((s) => s.id === id));
  const unattendedRisk = (systems ?? []).some((s) => picked.includes(s.id) && (s.mode === "read_only" || s.mode === "approve_each"));

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-db-dialog" role="dialog" aria-label={t.ac_sys_title(agentName)} data-testid="systems-skill-dialog">
        <h3 className="task-dialog-title ac-inline-icon">
          <span className="ac-agent-ico" aria-hidden>
            <RoleIcon name="globe" />
          </span>
          {t.ac_sys_title(agentName)}
        </h3>
        <p className="ac-dialog-desc">{t.ac_sys_desc}</p>

        {systems === null && !error && <p className="task-field-hint">{t.ac_db_loading}</p>}
        {systems?.length === 0 && <p className="task-field-hint ac-bad-text">{t.ac_sys_none_defined}</p>}
        {!!systems?.length && (
          <div className="ac-db-conns" data-testid="systems-skill-list">
            {systems.map((s) => (
              <label key={s.id} className="ac-db-option">
                <input
                  type="checkbox"
                  checked={picked.includes(s.id)}
                  disabled={!s.enabled && !picked.includes(s.id)}
                  onChange={(e) => setPicked((p) => (e.target.checked ? [...p, s.id] : p.filter((x) => x !== s.id)))}
                />
                <span>
                  {s.name} <span className="ac-db-meta">· {modeLabel(s.mode)}{!s.enabled ? ` · ${t.ac_sys_disabled}` : ""}</span>
                </span>
              </label>
            ))}
          </div>
        )}
        {missing.length > 0 && <p className="task-field-hint ac-bad-text">{t.ac_sys_missing(missing.join("、"))}</p>}
        {unattendedRisk && (
          <p className="task-field-hint" role="note" data-testid="systems-skill-warning">
            {t.ac_sys_unattended}
          </p>
        )}

        {error && <p className="ac-bad-text">{error}</p>}
        <div className="task-dialog-actions">
          {current.length > 0 && (
            <button className="aiterm-btn aiterm-btn--ghost ac-db-remove" disabled={busy} onClick={() => void save([])}>
              {t.ac_sys_remove}
            </button>
          )}
          <span className="ac-spacer" />
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.cancel}
          </button>
          <button className="aiterm-btn aiterm-btn--primary" disabled={busy || picked.length === 0} onClick={() => void save(picked)}>
            {t.ac_sys_save}
          </button>
        </div>
      </div>
    </div>
  );
}
