import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { DbSkill, DbSkillMode } from "../../ipc/agents";
import { dbListConnections, type DbConnectionInfo } from "../../ipc/db";
import { RoleIcon } from "./RoleIcon";

/** 資料庫技能的設定：開放哪些連線、權限三選一。`current` 為 null 是新裝。 */
export function DbSkillDialog({
  agentName,
  current,
  onSave,
  onClose,
}: {
  agentName: string;
  current: DbSkill | null;
  onSave: (skill: DbSkill | null) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const [conns, setConns] = useState<DbConnectionInfo[] | null>(null);
  const [picked, setPicked] = useState<string[]>(current?.connections ?? []);
  const [mode, setMode] = useState<DbSkillMode>(current?.mode ?? "read");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    dbListConnections()
      .then((c) => alive && setConns(c))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, []);

  const save = async (skill: DbSkill | null) => {
    setBusy(true);
    setError(null);
    try {
      await onSave(skill);
      onClose();
    } catch (e) {
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    } finally {
      setBusy(false);
    }
  };

  const modes: { value: DbSkillMode; label: string; hint: string }[] = [
    { value: "read", label: t.ac_db_mode_read, hint: t.ac_db_mode_read_hint },
    { value: "approve", label: t.ac_db_mode_approve, hint: t.ac_db_mode_approve_hint },
    { value: "write", label: t.ac_db_mode_write, hint: t.ac_db_mode_write_hint },
  ];

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-db-dialog" role="dialog" aria-label={t.ac_db_title(agentName)} data-testid="db-skill-dialog">
        <h3 className="task-dialog-title ac-inline-icon">
          <span className="ac-agent-ico ac-skill-ico-db" aria-hidden>
            <RoleIcon name="database" />
          </span>
          {t.ac_db_title(agentName)}
        </h3>
        <p className="ac-dialog-desc">{t.ac_db_desc}</p>

        <div className="ac-db-section">{t.ac_db_connections}</div>
        {conns === null && !error && <p className="task-field-hint">{t.ac_db_loading}</p>}
        {conns?.length === 0 && <p className="task-field-hint ac-bad-text">{t.ac_db_no_connections}</p>}
        {/* 連線多時只捲這一區，權限與按鈕永遠看得到。 */}
        {!!conns?.length && (
          <div className="ac-db-conns" data-testid="db-skill-conns">
            {conns.map((c) => (
              <label key={c.id} className="ac-db-option">
                <input
                  type="checkbox"
                  checked={picked.includes(c.id)}
                  onChange={(e) => setPicked((p) => (e.target.checked ? [...p, c.id] : p.filter((x) => x !== c.id)))}
                />
                <span>
                  {c.name} <span className="ac-db-meta">{[c.db_type, c.database].filter(Boolean).map((s) => `· ${s}`).join(" ")}</span>
                </span>
              </label>
            ))}
          </div>
        )}

        <div className="ac-db-section">{t.ac_db_permission}</div>
        {modes.map((m) => (
          <label key={m.value} className="ac-db-option">
            <input type="radio" name="ac-db-mode" checked={mode === m.value} onChange={() => setMode(m.value)} aria-label={m.label} />
            <span>
              {m.label} <span className="ac-db-meta">{m.hint}</span>
            </span>
          </label>
        ))}

        {error && <p className="ac-bad-text">{error}</p>}
        <div className="task-dialog-actions">
          {current && (
            <button className="aiterm-btn aiterm-btn--ghost ac-db-remove" disabled={busy} onClick={() => void save(null)}>
              {t.ac_db_remove}
            </button>
          )}
          <span className="ac-spacer" />
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.cancel}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary"
            disabled={busy || picked.length === 0}
            onClick={() => void save({ connections: picked, mode })}
          >
            {t.ac_db_save}
          </button>
        </div>
      </div>
    </div>
  );
}
