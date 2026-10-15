import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import type { AgentKind, AgentRow } from "../../ipc/agents";
import { usedDirs } from "../../ipc/projects";
import { rolesFor } from "./roles";

export interface AgentConfigValues {
  name: string;
  role: string;
  prompt_prefix: string;
  project_dir: string;
}

/**
 * 「指派角色」對話框。新建時才能選程式庫；編輯時程式庫鎖住——Agent 的
 * 分支已經從那個 repo 分出去了。送出失敗（例如不是 git repo）的錯誤顯示
 * 在對話框裡，對話框不關，使用者改完可以直接再送。
 */
export function AgentConfigDialog({
  projectId,
  kind,
  agent,
  defaultName,
  onCancel,
  onSubmit,
}: {
  projectId: string;
  kind: AgentKind;
  /** `null` = 新建。 */
  agent: AgentRow | null;
  defaultName: string;
  onCancel: () => void;
  onSubmit: (values: AgentConfigValues) => Promise<void>;
}) {
  const { t } = useLocale();
  const isNew = agent === null;
  const roles = rolesFor(kind);
  const [name, setName] = useState(agent?.name ?? defaultName);
  const [role, setRole] = useState(agent?.role ?? roles[0].name);
  const [prefix, setPrefix] = useState(agent?.prompt_prefix ?? roles[0].prefix);
  const [dir, setDir] = useState(agent?.project_dir ?? "");
  const [dirs, setDirs] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isNew) return;
    let alive = true;
    usedDirs(projectId)
      .then((d) => {
        if (!alive) return;
        setDirs(d);
        setDir((cur) => cur || d[0] || "");
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [isNew, projectId]);

  const pickRole = (r: (typeof roles)[number]) => {
    setRole(r.name);
    setPrefix(r.prefix);
  };

  const browse = async () => {
    try {
      const picked = await open({ directory: true, defaultPath: dir || undefined });
      if (typeof picked === "string") setDir(picked);
    } catch (e) {
      setError(String(e));
    }
  };

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      await onSubmit({ name: name.trim() || defaultName, role, prompt_prefix: prefix, project_dir: dir.trim() });
    } catch (e) {
      // Tauri 的 invoke 錯誤是字串；其他情況才是 Error。
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
      setBusy(false);
    }
  };

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog" role="dialog" aria-label={isNew ? t.ac_dialog_new_title : t.ac_dialog_edit_title}>
        <h3 className="task-dialog-title">{isNew ? t.ac_dialog_new_title : t.ac_dialog_edit_title}</h3>
        <p className="ac-dialog-desc">{t.ac_dialog_desc}</p>
        <label className="task-field">
          <span className="task-field-label">{t.ac_name}</span>
          <input className="task-field-input" value={name} onChange={(e) => setName(e.target.value)} autoFocus />
        </label>
        <div className="task-field">
          <span className="task-field-label">{t.ac_role}</span>
          <div className="ac-roles">
            {roles.map((r) => (
              <button
                key={r.name}
                type="button"
                className={`ac-role${r.name === role ? " ac-role--on" : ""}`}
                onClick={() => pickRole(r)}
              >
                {r.name}
              </button>
            ))}
          </div>
        </div>
        <label className="task-field">
          <span className="task-field-label">{t.ac_prefix}</span>
          <textarea
            className="task-field-input task-field-textarea"
            value={prefix}
            onChange={(e) => setPrefix(e.target.value)}
            rows={4}
          />
        </label>
        <div className="task-field">
          <span className="task-field-label">{t.ac_repo}</span>
          {isNew ? (
            <>
              <div className="task-field-row">
                <input
                  className="task-field-input task-field-input--mono"
                  value={dir}
                  onChange={(e) => setDir(e.target.value)}
                  aria-label={t.ac_repo}
                />
                <button type="button" className="aiterm-btn aiterm-btn--secondary" onClick={() => void browse()}>
                  {t.ac_browse}
                </button>
              </div>
              {dirs.length > 1 && (
                <div className="ac-roles">
                  {dirs.map((d) => (
                    <button key={d} type="button" className={`ac-role ac-role--mono${d === dir ? " ac-role--on" : ""}`} onClick={() => setDir(d)}>
                      {d}
                    </button>
                  ))}
                </div>
              )}
            </>
          ) : (
            <div className="ac-readonly" title={t.ac_repo_locked}>
              {agent.project_dir}
              {agent.branch ? ` · ${agent.branch}` : ""}
            </div>
          )}
        </div>
        {error && (
          <p className="task-field-error" role="alert">
            {error}
          </p>
        )}
        <div className="ac-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" disabled={busy} onClick={onCancel}>
            {t.ac_cancel}
          </button>
          <button className="aiterm-btn aiterm-btn--primary" disabled={busy || (isNew && !dir.trim())} onClick={() => void submit()}>
            {isNew ? t.ac_place : t.ac_save}
          </button>
        </div>
      </div>
    </div>
  );
}
