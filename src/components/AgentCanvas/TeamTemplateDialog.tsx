import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { checkRepo, createProjectRepo, projectRepo, setProjectRepo, type AgentRow, type ProjectRepo, type RepoCheck } from "../../ipc/agents";
import { RepoStatus } from "./RepoStatus";
import { confirmInitRepo } from "./repoInit";
import { builtinTemplates, deleteTemplate, loadTemplates, saveTemplate, snapshot, type TeamTemplate } from "./teamTemplates";

/**
 * 團隊範本：內建五組常用團隊，加上自己存的範本，直接套用到這個專案的程式庫——
 * 程式庫依序取桌布上 Agent 的、專案記住的、看板資料夾所在的；都沒有才請使用者選一次
 * （選過就記住）。還不是 git／沒有 commit 的話這裡就能一鍵初始化：套用範本通常是在
 * 空桌布上，沒有別的地方可以初始化。
 */
export function TeamTemplateDialog({
  projectId,
  agents,
  onApply,
  onClose,
}: {
  projectId: string;
  agents: AgentRow[];
  onApply: (tpl: TeamTemplate, repoDir: string) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const builtins = builtinTemplates(t);
  const [templates, setTemplates] = useState<TeamTemplate[]>(() => loadTemplates());
  const [name, setName] = useState("");
  /** 專案的程式庫；undefined = 還在查，null = 查不到（舊版後端等）。 */
  const [found, setFound] = useState<ProjectRepo | null | undefined>(undefined);
  const resolved = found === undefined ? undefined : (found?.path ?? null);
  /** 使用者自己選或改過的程式庫（選過才需要記住）。 */
  const [picked, setPicked] = useState<string | null>(null);
  const [changing, setChanging] = useState(false);
  const repo = (picked ?? resolved ?? "").trim();
  const [check, setCheck] = useState<RepoCheck | null>(null);
  const [checkRev, setCheckRev] = useState(0);
  const [initing, setIniting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    projectRepo(projectId)
      .then((r) => alive && setFound(r))
      .catch(() => alive && setFound(null));
    return () => {
      alive = false;
    };
  }, [projectId]);

  useEffect(() => {
    let alive = true;
    if (!repo) return;
    checkRepo(repo)
      .then((c) => alive && setCheck(c))
      .catch(() => alive && setCheck(null));
    return () => {
      alive = false;
    };
  }, [repo, checkRev]);
  // 預設的 workspace 還沒建立（資料夾不存在）：套用時才建立並初始化，所以也算可以套用。
  const willCreate = !picked && found?.source === "default" && check?.status === "missing";
  const repoOk = !!repo && (check?.status === "ok" || willCreate);
  // 沒有程式庫或使用者要換：顯示選擇；否則只顯示「套用到：<路徑>」。
  const showPicker = resolved !== undefined && (!resolved || changing);

  const fixRepo = async () => {
    setIniting(true);
    setError(null);
    setError(await confirmInitRepo(t, repo, check?.status, projectId));
    setIniting(false);
    setCheckRev((n) => n + 1);
  };

  const pick = async () => {
    try {
      const p = await open({ directory: true, defaultPath: repo || undefined });
      if (typeof p === "string") {
        setCheck(null);
        setPicked(p);
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const apply = async (tpl: TeamTemplate) => {
    setBusy(true);
    setError(null);
    try {
      if (willCreate) await createProjectRepo(projectId, repo);
      await onApply(tpl, repo);
      // 自己選的程式庫記在專案裡，下次不用再選。
      if (picked && picked.trim() !== resolved) await setProjectRepo(projectId, repo);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const applyButton = (tpl: TeamTemplate) => (
    <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={!repoOk || busy} onClick={() => void apply(tpl)}>
      {t.ac_tpl_apply}
    </button>
  );

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-tpl-dialog" role="dialog" aria-label={t.ac_tpl_title}>
        <h3 className="task-dialog-title">{t.ac_tpl_title}</h3>
        <p className="ac-dialog-desc">{t.ac_tpl_desc}</p>

        <div className="task-field">
          <span className="task-field-label">{t.ac_tpl_target}</span>
          {showPicker ? (
            <>
              <div className="ac-tpl-row">
                <input
                  className="task-field-input"
                  value={picked ?? resolved ?? ""}
                  aria-label={t.ac_tpl_repo}
                  onChange={(e) => {
                    setCheck(null);
                    setPicked(e.target.value);
                  }}
                />
                <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void pick()}>
                  {t.ac_tpl_browse}
                </button>
              </div>
              {!repo && <span className="task-field-hint">{t.ac_tpl_repo_pick}</span>}
            </>
          ) : (
            <div className="ac-tpl-row ac-tpl-target">
              <code className="ac-tpl-path">{repo}</code>
              {resolved !== undefined && (
                <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => setChanging(true)}>
                  {t.ac_tpl_change_repo}
                </button>
              )}
            </div>
          )}
          {willCreate ? (
            <span className="task-field-hint">{t.ac_tpl_will_create}</span>
          ) : (
            repo && check && <RepoStatus repo={check} busy={initing} onFix={() => void fixRepo()} />
          )}
        </div>

        <div className="ac-tpl-section">{t.ac_tpl_builtin}</div>
        <div className="ac-tpl-list">
          {builtins.map((tpl) => (
            <div key={tpl.id} className="ac-tpl-item" data-testid={`tpl-builtin-${tpl.id}`}>
              <div>
                <b>{tpl.name}</b>
                <small className="ac-tpl-desc">{tpl.description}</small>
                <small>{tpl.agents.map((a) => a.role || a.name).join("、")}</small>
              </div>
              {applyButton(tpl)}
            </div>
          ))}
        </div>

        <div className="ac-tpl-section">{t.ac_tpl_mine}</div>
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
                {applyButton(tpl)}
                <button
                  className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                  aria-label={t.ac_tpl_delete(tpl.name)}
                  title={t.ac_tpl_delete(tpl.name)}
                  onClick={() => setTemplates(deleteTemplate(tpl.name))}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        )}
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
