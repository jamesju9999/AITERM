import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { invokeAiComplete } from "../../ipc/ai";
import { assignAgent, checkRepo, createAgent, getReferences, setProjectRepo, type RepoCheck } from "../../ipc/agents";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { createTask, getProjectBrief, setProjectBrief } from "../../ipc/tasks";
import { ModelPickerButton } from "../ModelPickerButton";
import { useRefineTask } from "../TaskBoard/useRefineTask";
import { restoreWebviewFocus } from "../../lib/webviewFocus";
import { RepoStatus } from "./RepoStatus";
import { confirmInitRepo } from "./repoInit";
import { startFromRequirement, StartError, type StartDeps, type StartResult } from "./startFromRequirement";

/** 跟專案簡介、範本推薦共用模型偏好。 */
const REFINE_PROVIDER_KEY = "aiterm_refine_provider";

const deps: StartDeps = {
  getBrief: getProjectBrief,
  setBrief: setProjectBrief,
  getReferences,
  complete: async (messages, system, streamId, providerId) => (await invokeAiComplete(messages, system, streamId, providerId)).content ?? "",
  createAgent,
  createTask,
  assignAgent,
};

/**
 * 空桌布上的「你想做什麼？」：打需求就依需求組好團隊、把需求派給第一站。
 * 程式庫沿用專案記住的；沒有就在這裡選（選了就記住）。
 */
export function RequirementStart({
  projectId,
  repo,
  onRepoChange,
  onStarted,
}: {
  projectId: string;
  /** 專案記住的程式庫；null = 沒有。 */
  repo: string | null;
  onRepoChange: (path: string) => void;
  onStarted: (r: StartResult) => void;
}) {
  const { t } = useLocale();
  const [text, setText] = useState("");
  const [check, setCheck] = useState<RepoCheck | null>(null);
  const [checkRev, setCheckRev] = useState(0);
  const [initing, setIniting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState<string | undefined>(undefined);
  const refiner = useRefineTask();
  /** 潤飾前的內容；有值才顯示「還原」。 */
  const [beforeRefine, setBeforeRefine] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    listProviders()
      .then((list) => {
        if (!alive) return;
        setProviders(list);
        const remembered = localStorage.getItem(REFINE_PROVIDER_KEY);
        setProviderId(remembered && list.some((p) => p.id === remembered) ? remembered : (list.find((p) => p.is_default)?.id ?? list[0]?.id));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

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

  const repoOk = !!repo && check?.status === "ok";

  const pick = async () => {
    try {
      const p = await open({ directory: true, defaultPath: repo || undefined });
      await restoreWebviewFocus();
      if (typeof p === "string") {
        setCheck(null);
        await setProjectRepo(projectId, p);
        onRepoChange(p);
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const fixRepo = async () => {
    if (!repo) return;
    setIniting(true);
    setError(await confirmInitRepo(t, repo, check?.status, projectId));
    setIniting(false);
    setCheckRev((n) => n + 1);
  };

  const refine = async () => {
    const draft = text.trim();
    if (!draft) return;
    setError(null);
    const r = await refiner.refine(draft, repo ?? "", false, providerId);
    if (!r) return;
    if (providerId) localStorage.setItem(REFINE_PROVIDER_KEY, providerId);
    setBeforeRefine(text);
    setText(r.body);
  };

  const submit = async () => {
    if (!repo || !text.trim()) return;
    setBusy(true);
    setError(null);
    try {
      onStarted(await startFromRequirement({ projectId, requirement: text, repoDir: repo, t, providerId }, deps));
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      // 團隊已經建好：需求框會隨桌布不再空而消失，要告訴使用者接下來怎麼做。
      setError(e instanceof StartError && e.teamCreated ? t.ac_req_team_exists(msg) : msg);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="ac-req" data-testid="ac-req">
      <h3 className="ac-req-title">{t.ac_req_title}</h3>
      <p className="ac-req-sub">{t.ac_req_sub}</p>
      <textarea
        className="ac-req-input"
        data-testid="ac-req-input"
        rows={4}
        value={text}
        placeholder={t.ac_req_placeholder}
        disabled={busy || refiner.busy}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void submit();
        }}
      />
      <div className="ac-req-tools">
        {providers.length > 0 && (
          <ModelPickerButton providers={providers} selectedId={providerId ?? ""} onChange={setProviderId} disabled={refiner.busy || busy} />
        )}
        <button
          type="button"
          className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
          data-testid="ac-req-refine"
          title={t.ac_req_refine_hint}
          disabled={busy || refiner.busy || !text.trim() || providers.length === 0}
          onClick={() => void refine()}
        >
          {refiner.busy ? t.board_refine_busy : t.board_refine}
        </button>
        {beforeRefine !== null && (
          <button
            type="button"
            className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
            data-testid="ac-req-refine-undo"
            disabled={refiner.busy}
            onClick={() => {
              setText(beforeRefine);
              setBeforeRefine(null);
            }}
          >
            {t.board_refine_undo}
          </button>
        )}
      </div>
      <div className="ac-req-repo">
        <span className="ac-req-repo-label">{t.ac_req_repo_label}</span>
        {repo ? (
          <code className="ac-req-path" title={repo}>
            {repo}
          </code>
        ) : (
          <span className="ac-req-path ac-req-path--empty">{t.ac_req_repo_pick}</span>
        )}
        <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" disabled={busy} onClick={() => void pick()}>
          {repo ? t.ac_tpl_change_repo : t.ac_tpl_browse}
        </button>
      </div>
      {repo && check && <RepoStatus repo={check} busy={initing} onFix={() => void fixRepo()} />}
      {providerId === undefined && <span className="task-field-hint">{t.ac_req_no_ai}</span>}
      {(error ?? refiner.error) && <p className="ac-bad-text">{error ?? refiner.error}</p>}
      <div className="ac-req-actions">
        <span className="ac-req-shortcut">{t.ac_req_shortcut}</span>
        <button
          className="aiterm-btn aiterm-btn--primary"
          data-testid="ac-req-send"
          disabled={busy || !text.trim() || !repoOk}
          onClick={() => void submit()}
        >
          {busy ? t.ac_req_busy : t.ac_req_send}
        </button>
      </div>
    </div>
  );
}
