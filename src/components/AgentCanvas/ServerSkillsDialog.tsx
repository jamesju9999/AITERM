import { confirm, open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { useIntakeRemote } from "../../hooks/useIntakeRemote";
import { readSkill } from "../../ipc/agents";
import { deleteServerSkill, listServerSkills, uploadServerSkill, type ServerSkill } from "../../ipc/serverSkills";
import { RoleIcon } from "./RoleIcon";

/** 一位 Agent 最多幾個自訂技能（server 端也擋，這裡先提示）。 */
const MAX_PICKED = 12;

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

/**
 * 共用專案的 Agent 技能：從 server 技能庫勾選。管理員可以從本機資料夾上傳新技能、刪除技能並儲存勾選；
 * 其他成員只能看（server 也只收管理員的變更）。
 */
export function ServerSkillsDialog({
  agentName,
  current,
  onSave,
  onClose,
}: {
  agentName: string;
  /** 目前已裝的技能 id。 */
  current: string[];
  onSave: (ids: string[]) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const isAdmin = useIntakeRemote().role === "admin";
  const [skills, setSkills] = useState<ServerSkill[] | null>(null);
  const [picked, setPicked] = useState<string[]>(current);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    listServerSkills()
      .then((s) => alive && setSkills(s))
      .catch((e) => {
        if (!alive) return;
        setSkills([]);
        setError(errText(e));
      });
    return () => {
      alive = false;
    };
  }, []);

  const addFromFolder = async () => {
    try {
      const dir = await open({ directory: true });
      if (typeof dir !== "string" || !dir) return;
      // 選的可能是技能本身、含多個技能的資料夾，或程式庫根目錄：全部找出來上傳。
      const found = await readSkill(dir);
      if (found.length === 0) {
        setError(t.ac_ss_none_found);
        return;
      }
      setBusy(true);
      setError(null);
      const added: ServerSkill[] = [];
      for (const f of found) added.push(await uploadServerSkill(f.path));
      const list = await listServerSkills();
      setSkills(list);
      setPicked((p) => [...p, ...added.map((a) => a.id).filter((id) => !p.includes(id))].slice(0, MAX_PICKED));
      setNotice(t.ac_ss_uploaded(added.map((a) => a.name)));
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (s: ServerSkill) => {
    if (!(await confirm(t.ac_ss_delete_confirm(s.name), { kind: "warning" }))) return;
    try {
      await deleteServerSkill(s.id);
      setSkills((cur) => (cur ?? []).filter((x) => x.id !== s.id));
      setPicked((p) => p.filter((id) => id !== s.id));
      setError(null);
    } catch (e) {
      setError(errText(e));
    }
  };

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      await onSave((skills ?? []).map((s) => s.id).filter((id) => picked.includes(id)));
      onClose();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const toggle = (id: string, on: boolean) => {
    if (on && picked.length >= MAX_PICKED) {
      setError(t.ac_ss_limit(MAX_PICKED));
      return;
    }
    setError(null);
    setPicked((p) => (on ? [...p, id] : p.filter((x) => x !== id)));
  };

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-db-dialog" role="dialog" aria-label={t.ac_ss_title(agentName)} data-testid="server-skills-dialog">
        <h3 className="task-dialog-title ac-inline-icon">
          <span className="ac-agent-ico ac-skill-ico-db" aria-hidden>
            <RoleIcon name="doc" />
          </span>
          {t.ac_ss_title(agentName)}
        </h3>
        <p className="ac-dialog-desc">{t.ac_ss_desc}</p>
        {skills === null && <p className="task-field-hint">{t.ac_db_loading}</p>}
        {skills?.length === 0 && !error && <p className="task-field-hint">{isAdmin ? t.ac_ss_none_admin : t.ac_ss_none_member}</p>}
        <div className="ac-db-conns">
          {(skills ?? []).map((s) => (
            <div key={s.id} className="ac-db-option ac-ss-row">
              <label className="ac-ss-label" title={s.name}>
                <input type="checkbox" checked={picked.includes(s.id)} disabled={!isAdmin} onChange={(e) => toggle(s.id, e.target.checked)} aria-label={s.name} />
                <span className="ac-cs-text">
                  <span className="ac-cs-name">{s.name}</span>
                  {s.description && (
                    <span className="ac-db-meta ac-cs-desc" title={s.description}>
                      {s.description}
                    </span>
                  )}
                  <span className="ac-db-meta">{t.ac_ss_meta(s.files, Math.max(1, Math.round(s.bytes / 1024)), s.uploaded_by)}</span>
                </span>
              </label>
              {isAdmin && (
                <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void remove(s)} aria-label={`${t.ac_ss_delete} ${s.name}`}>
                  {t.ac_ss_delete}
                </button>
              )}
            </div>
          ))}
        </div>
        {isAdmin ? (
          <>
            <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm ac-refs-add" disabled={busy} onClick={() => void addFromFolder()}>
              ＋ {t.ac_ss_add}
            </button>
            <p className="task-field-hint">{t.ac_ss_add_hint}</p>
          </>
        ) : (
          <p className="task-field-hint">{t.ac_ss_readonly}</p>
        )}
        {notice && !error && <p className="ac-good-text">{notice}</p>}
        {error && (
          <p className="ac-bad-text" role="alert">
            {error}
          </p>
        )}
        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {isAdmin ? t.cancel : t.ac_done_close}
          </button>
          {isAdmin && (
            <button className="aiterm-btn aiterm-btn--primary" disabled={busy || skills === null} onClick={() => void save()}>
              {t.ac_db_save}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
