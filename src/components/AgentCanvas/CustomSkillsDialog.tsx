import { open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { listSkills, readSkill, type SkillInfo } from "../../ipc/agents";
import { RoleIcon } from "./RoleIcon";

/** 選這位 Agent 的自訂技能：程式庫的、自己的，或從任何含 SKILL.md 的資料夾加入。 */
export function CustomSkillsDialog({
  agentName,
  repoDir,
  current,
  onSave,
  onClose,
}: {
  agentName: string;
  repoDir: string;
  /** 目前已裝的技能資料夾路徑。 */
  current: string[];
  onSave: (paths: string[]) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const [skills, setSkills] = useState<SkillInfo[] | null>(null);
  const [picked, setPicked] = useState<string[]>(current);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    (async () => {
      const found = await listSkills(repoDir).catch(() => [] as SkillInfo[]);
      // 已裝但不在兩個預設位置的（從資料夾加入的），也要列出來。
      const extra = await Promise.all(
        current.filter((p) => !found.some((s) => s.path === p)).map((p) => readSkill(p).catch(() => [] as SkillInfo[])),
      );
      if (alive) setSkills([...found, ...extra.flat().filter((s) => current.includes(s.path))]);
    })();
    return () => {
      alive = false;
    };
  }, [repoDir, current]);

  const addFolder = async () => {
    try {
      const dir = await open({ directory: true });
      if (typeof dir !== "string" || !dir) return;
      // 可能一次找到好幾個（選了程式庫根目錄或 skills 資料夾）：全部列出並勾選。
      const found = await readSkill(dir);
      setSkills((s) => [...(s ?? []), ...found.filter((f) => !(s ?? []).some((x) => x.path === f.path)).map((f) => ({ ...f, source: "folder" as const }))]);
      setPicked((p) => [...p, ...found.map((f) => f.path).filter((x) => !p.includes(x))]);
      setError(null);
    } catch (e) {
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    }
  };

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      // 依清單順序存，不是勾選順序：提示裡的順序穩定。
      await onSave((skills ?? []).map((s) => s.path).filter((p) => picked.includes(p)));
      onClose();
    } catch (e) {
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    } finally {
      setBusy(false);
    }
  };

  const groups: { source: SkillInfo["source"]; label: string }[] = [
    { source: "repo", label: t.ac_cs_from_repo },
    { source: "user", label: t.ac_cs_from_user },
    { source: "folder", label: t.ac_cs_from_folder },
  ];

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-db-dialog" role="dialog" aria-label={t.ac_cs_title(agentName)} data-testid="custom-skills-dialog">
        <h3 className="task-dialog-title ac-inline-icon">
          <span className="ac-agent-ico ac-skill-ico-db" aria-hidden>
            <RoleIcon name="doc" />
          </span>
          {t.ac_cs_title(agentName)}
        </h3>
        <p className="ac-dialog-desc">{t.ac_cs_desc}</p>
        {skills === null && <p className="task-field-hint">{t.ac_db_loading}</p>}
        {skills?.length === 0 && <p className="task-field-hint">{t.ac_cs_none}</p>}
        <div className="ac-db-conns">
          {groups.map((g) => {
            const list = (skills ?? []).filter((s) => (g.source === "folder" ? s.source === "folder" || s.source === "agent" : s.source === g.source));
            if (list.length === 0) return null;
            return (
              <div key={g.source}>
                <div className="ac-db-section">{g.label}</div>
                {list.map((s) => (
                  <label key={s.path} className="ac-db-option" title={s.path}>
                    <input
                      type="checkbox"
                      checked={picked.includes(s.path)}
                      onChange={(e) => setPicked((p) => (e.target.checked ? [...p, s.path] : p.filter((x) => x !== s.path)))}
                      aria-label={s.name}
                    />
                    <span className="ac-cs-text">
                      <span className="ac-cs-name">{s.name}</span>
                      {s.description && (
                        <span className="ac-db-meta ac-cs-desc" title={s.description}>
                          {s.description}
                        </span>
                      )}
                    </span>
                  </label>
                ))}
              </div>
            );
          })}
        </div>
        <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm ac-refs-add" onClick={() => void addFolder()}>
          ＋ {t.ac_cs_add_folder}
        </button>
        <p className="task-field-hint">{t.ac_cs_folder_hint}</p>
        {error && <p className="ac-bad-text">{error}</p>}
        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.cancel}
          </button>
          <button className="aiterm-btn aiterm-btn--primary" disabled={busy || skills === null} onClick={() => void save()}>
            {t.ac_db_save}
          </button>
        </div>
      </div>
    </div>
  );
}
