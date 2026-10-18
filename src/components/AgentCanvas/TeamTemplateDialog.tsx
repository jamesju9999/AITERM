import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { checkRepo, type AgentRow, type RepoCheck } from "../../ipc/agents";
import { RepoStatus } from "./RepoStatus";
import { confirmInitRepo } from "./repoInit";
import { deleteTemplate, loadTemplates, saveTemplate, snapshot, type TeamTemplate } from "./teamTemplates";

/**
 * 團隊範本：把桌布上的團隊存起來，或把存好的團隊一次放到這個專案。套用時所有 Agent
 * 用同一個程式庫（要是已經有 commit 的 git repo；還不是的話這裡就能一鍵初始化——
 * 套用範本通常是在空桌布上，沒有別的地方可以初始化）。
 */
export function TeamTemplateDialog({
  projectId,
  agents,
  defaultRepo,
  onApply,
  onClose,
}: {
  projectId: string;
  agents: AgentRow[];
  defaultRepo: string;
  onApply: (tpl: TeamTemplate, repoDir: string) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const [templates, setTemplates] = useState<TeamTemplate[]>(() => loadTemplates());
  const [name, setName] = useState("");
  // 使用者還沒改過就跟著 defaultRepo（對話框可能在 Agent 載入完之前就打開了）。
  const [edited, setEdited] = useState<string | null>(null);
  const repo = edited ?? defaultRepo;
  const setRepo = setEdited;
  const [check, setCheck] = useState<RepoCheck | null>(null);
  const [checkRev, setCheckRev] = useState(0);
  const [initing, setIniting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    if (!repo.trim()) return;
    checkRepo(repo.trim())
      .then((c) => alive && setCheck(c))
      .catch(() => alive && setCheck(null));
    return () => {
      alive = false;
    };
  }, [repo, checkRev]);
  const repoOk = !!repo.trim() && check?.status === "ok";

  const fixRepo = async () => {
    setIniting(true);
    setError(null);
    setError(await confirmInitRepo(t, repo.trim(), check?.status, projectId));
    setIniting(false);
    setCheckRev((n) => n + 1);
  };

  const pick = async () => {
    try {
      const p = await open({ directory: true, defaultPath: repo || undefined });
      if (typeof p === "string") {
        setCheck(null);
        setRepo(p);
      }
    } catch (e) {
      setError(String(e));
    }
  };
  const apply = async (tpl: TeamTemplate) => {
    setBusy(true);
    setError(null);
    try {
      await onApply(tpl, repo.trim());
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog" role="dialog" aria-label={t.ac_tpl_title}>
        <h3 className="task-dialog-title">{t.ac_tpl_title}</h3>
        <p className="ac-dialog-desc">{t.ac_tpl_desc}</p>

        <div className="ac-tpl-row">
          <input
            className="task-field-input"
            value={name}
            placeholder={t.ac_tpl_name}
            aria-label={t.ac_tpl_name}
            onChange={(e) => setName(e.target.value)}
          />
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            disabled={!name.trim() || agents.length === 0}
            title={agents.length === 0 ? t.ac_tpl_empty_canvas : undefined}
            onClick={() => {
              setTemplates(saveTemplate(snapshot(name.trim(), agents)));
              setName("");
            }}
          >
            {t.ac_tpl_save}
          </button>
        </div>

        <label className="task-field">
          <span className="task-field-label">{t.ac_tpl_repo}</span>
          <div className="ac-tpl-row">
            <input
              className="task-field-input"
              value={repo}
              aria-label={t.ac_tpl_repo}
              onChange={(e) => {
                setCheck(null);
                setRepo(e.target.value);
              }}
            />
            <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void pick()}>
              {t.ac_tpl_browse}
            </button>
          </div>
          {repo.trim() && check && <RepoStatus repo={check} busy={initing} onFix={() => void fixRepo()} />}
          {!repo.trim() && <span className="task-field-hint">{t.ac_tpl_repo_pick}</span>}
        </label>

        {templates.length === 0 ? (
          <p className="ac-dialog-desc">{t.ac_tpl_none}</p>
        ) : (
          <div className="ac-tpl-list">
            {templates.map((tpl) => (
              <div key={tpl.name} className="ac-tpl-item" data-testid={`tpl-${tpl.name}`}>
                <div>
                  <b>{tpl.name}</b>
                  <small>{tpl.agents.map((a) => a.name).join("、")}</small>
                </div>
                <button
                  className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                  disabled={!repoOk || busy}
                  onClick={() => void apply(tpl)}
                >
                  {t.ac_tpl_apply}
                </button>
                <button
                  className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                  aria-label={t.ac_tpl_delete(tpl.name)}
                  onClick={() => setTemplates(deleteTemplate(tpl.name))}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        )}
        {error && <p className="ac-bad-text">{error}</p>}
        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.ac_verify_close}
          </button>
        </div>
      </div>
    </div>
  );
}
