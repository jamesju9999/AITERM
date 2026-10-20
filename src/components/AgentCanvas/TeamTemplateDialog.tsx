import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { invokeAiComplete } from "../../ipc/ai";
import { checkRepo, getReferences, projectRepo, setProjectRepo, type AgentRow, type ProjectRepo, type RepoCheck } from "../../ipc/agents";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { getProjectBrief } from "../../ipc/tasks";
import { ModelPickerButton } from "../ModelPickerButton";
import { buildRecommendMessage, buildRecommendSystem, parseRecommendation, type Recommendation } from "./teamRecommend";
import { RepoStatus } from "./RepoStatus";
import { confirmInitRepo } from "./repoInit";
import { KindIcon } from "./KindIcon";
import {
  builtinTemplates,
  composition,
  deleteTemplate,
  loadTemplates,
  saveTemplate,
  snapshot,
  type BuiltinTemplate,
  type TeamTemplate,
} from "./teamTemplates";

/** 跟專案簡介、工作卡的「AI 潤飾」共用模型偏好。 */
const REFINE_PROVIDER_KEY = "aiterm_refine_provider";
const REC_KEY = "rec";
const recStoreKey = (projectId: string) => `aiterm.agentCanvas.teamRec.${projectId}`;

function loadRec(projectId: string): { brief: string; reply: string } | null {
  try {
    const v: unknown = JSON.parse(localStorage.getItem(recStoreKey(projectId)) ?? "null");
    const o = v as { brief?: unknown; reply?: unknown } | null;
    return o && typeof o.brief === "string" && typeof o.reply === "string" ? { brief: o.brief, reply: o.reply } : null;
  } catch {
    return null;
  }
}

function saveRec(projectId: string, rec: { brief: string; reply: string }) {
  try {
    localStorage.setItem(recStoreKey(projectId), JSON.stringify(rec));
  } catch {
    // 存不了就只留在這次的畫面
  }
}

/**
 * 團隊範本：內建的常用團隊，加上自己存的範本，先選一組再套用到這個專案的程式庫——加到目前
 * 桌布，或建立成新的編制。程式庫一律是
 * 使用者自己指定的：桌布上 Agent 用的、或這個專案記住的（在這裡選過一次就記住）；都沒有
 * 就請使用者選，系統不自己猜、不自己建立。還不是 git／沒有 commit 的話這裡就能一鍵
 * 初始化：套用範本通常是在空桌布上，沒有別的地方可以初始化。
 */
export function TeamTemplateDialog({
  projectId,
  agents,
  onApply,
  onApplyAsFormation,
  onRepoChange,
  onClose,
}: {
  projectId: string;
  agents: AgentRow[];
  /** 使用者換了專案資料夾（已記住）：讓協作頁的「新增模組」也跟著帶新的。 */
  onRepoChange?: (path: string) => void;
  onApply: (tpl: TeamTemplate, repoDir: string) => Promise<void>;
  /** 用範本建立一組新編制並切換過去（原本的團隊留在原來的編制）。 */
  onApplyAsFormation?: (tpl: TeamTemplate, repoDir: string) => Promise<void>;
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
  const repoOk = !!repo && check?.status === "ok";
  // 桌布上已有 Agent：鎖定為它們的程式庫。一個團隊只能用一個程式庫——協調者只能合併
  // 同一個程式庫裡的分支（實機：換了資料夾再套用，AIMailer 的團隊就分到兩個程式庫）。
  const locked = found?.source === "agents";
  // 沒有程式庫或使用者要換：顯示選擇；否則只顯示路徑。
  const showPicker = resolved !== undefined && !locked && (!resolved || changing);

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
        // 選了就記住，不必等套用：關掉對話框再打開、新增模組都會用新的。
        await setProjectRepo(projectId, p);
        onRepoChange?.(p);
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const apply = async (tpl: TeamTemplate, asFormation = false) => {
    setBusy(true);
    setError(null);
    try {
      if (asFormation && onApplyAsFormation) await onApplyAsFormation(tpl, repo);
      else await onApply(tpl, repo);
      // 手動輸入的路徑在套用時記住（用「選擇…」挑的已經記住了）。
      if (picked && picked.trim() !== resolved) {
        await setProjectRepo(projectId, repo);
        onRepoChange?.(repo);
      }
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // 選中的範本：內建用 id、我的範本用名稱（名稱在「我的範本」裡唯一）。
  const [selected, setSelected] = useState<string | null>(null);

  // ── 依專案簡介推薦 ─────────────────────────────────────────────────────
  const [brief, setBrief] = useState<string | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState("");
  const [recBusy, setRecBusy] = useState(false);
  const [recError, setRecError] = useState<string | null>(null);
  /** 上次的推薦（存 AI 原始回覆，換語系重新解析）與當時的簡介；關掉對話框再打開還在。 */
  const [recRaw, setRecRaw] = useState<{ brief: string; reply: string } | null>(() => loadRec(projectId));
  const rec: Recommendation | null = recRaw ? parseRecommendation(recRaw.reply, t) : null;
  const recStale = !!recRaw && brief !== null && recRaw.brief !== brief.trim();

  useEffect(() => {
    let alive = true;
    getProjectBrief(projectId)
      .then((b) => alive && setBrief(b ?? ""))
      .catch(() => alive && setBrief(""));
    // 跟專案簡介的「AI 潤飾」共用模型偏好：記住上次選的 → is_default → 清單第一個。
    listProviders()
      .then((list) => {
        if (!alive) return;
        setProviders(list);
        const remembered = localStorage.getItem(REFINE_PROVIDER_KEY);
        setProviderId(remembered && list.some((p) => p.id === remembered) ? remembered : (list.find((p) => p.is_default)?.id ?? list[0]?.id ?? ""));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [projectId]);

  const recommend = async () => {
    const b = (brief ?? "").trim();
    if (!b) return;
    setRecBusy(true);
    setRecError(null);
    try {
      const refs = await getReferences(projectId).catch(() => []);
      const reply = await invokeAiComplete(
        [{ role: "user", content: buildRecommendMessage(b, refs) }],
        buildRecommendSystem(t),
        "team-recommend",
        providerId || undefined,
      );
      const text = reply.content ?? "";
      if (!parseRecommendation(text, t)) {
        setRecError(t.ac_rec_err_parse);
        return;
      }
      if (providerId) localStorage.setItem(REFINE_PROVIDER_KEY, providerId);
      const next = { brief: b, reply: text };
      setRecRaw(next);
      saveRec(projectId, next);
      setSelected(REC_KEY);
    } catch (e) {
      setRecError(String(e));
    } finally {
      setRecBusy(false);
    }
  };
  const mine = (name: string) => `mine:${name}`;
  const chosenBuiltin: BuiltinTemplate | undefined = builtins.find((b) => b.id === selected);
  const chosenRec = selected === REC_KEY ? rec : null;
  const chosen: TeamTemplate | null =
    chosenRec?.template ?? chosenBuiltin ?? templates.find((m) => mine(m.name) === selected) ?? null;

  const card = (key: string, tpl: TeamTemplate, testId: string, onDelete?: () => void) => {
    const parts = composition(tpl);
    const on = selected === key;
    return (
      <div key={key} className={`ac-tpl-card${on ? " ac-tpl-card--on" : ""}`} data-testid={testId}>
        <button type="button" className="ac-tpl-card-main" aria-pressed={on} onClick={() => setSelected(key)}>
          <b>{tpl.name}</b>
          <small>{parts.map((p) => (p.count > 1 ? `${p.label} ×${p.count}` : p.label)).join(" · ")}</small>
        </button>
        {onDelete && (
          <button
            type="button"
            className="ac-tpl-card-del"
            aria-label={t.ac_tpl_delete(tpl.name)}
            title={t.ac_tpl_delete(tpl.name)}
            onClick={() => {
              if (on) setSelected(null);
              onDelete();
            }}
          >
            ×
          </button>
        )}
      </div>
    );
  };

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-tpl-dialog" role="dialog" aria-label={t.ac_tpl_title}>
        <div className="ac-tpl-body">
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
                <code className="ac-tpl-path" title={repo}>
                  {repo}
                </code>
                {resolved !== undefined && !locked && (
                  <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => setChanging(true)}>
                    {t.ac_tpl_change_repo}
                  </button>
                )}
              </div>
            )}
            {locked && <span className="task-field-hint">{t.ac_tpl_repo_locked}</span>}
            {repo && check && <RepoStatus repo={check} busy={initing} onFix={() => void fixRepo()} />}
          </div>

          <div className="ac-tpl-section">{t.ac_rec_section}</div>
          <div className="ac-rec" data-testid="tpl-recommend">
            <div className="ac-tpl-row">
              <span className="task-field-hint ac-rec-hint">
                {brief === "" ? t.ac_rec_need_brief : providers.length === 0 ? t.ac_rec_need_provider : t.ac_rec_hint}
              </span>
              {providers.length > 0 && (
                <ModelPickerButton providers={providers} selectedId={providerId} onChange={setProviderId} disabled={recBusy} />
              )}
              <button
                className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                disabled={!brief?.trim() || providers.length === 0 || recBusy}
                onClick={() => void recommend()}
              >
                {recBusy ? t.ac_rec_busy : rec ? t.ac_rec_again : t.ac_rec_button}
              </button>
            </div>
            {recError && <p className="ac-bad-text">{recError}</p>}
            {rec && (
              <div className="ac-tpl-grid">
                {card(REC_KEY, rec.template, "tpl-rec")}
              </div>
            )}
            {rec && recStale && <span className="task-field-hint ac-rec-stale">{t.ac_rec_stale}</span>}
          </div>

          <div className="ac-tpl-section">{t.ac_tpl_builtin}</div>
          <div className="ac-tpl-grid">{builtins.map((tpl) => card(tpl.id, tpl, `tpl-builtin-${tpl.id}`))}</div>

          <div className="ac-tpl-section">{t.ac_tpl_mine}</div>
          {templates.length === 0 ? (
            <p className="ac-dialog-desc">{t.ac_tpl_none}</p>
          ) : (
            <div className="ac-tpl-grid">
              {templates.map((tpl) => card(mine(tpl.name), tpl, `tpl-${tpl.name}`, () => setTemplates(deleteTemplate(tpl.name))))}
            </div>
          )}
          <div className="ac-tpl-row ac-tpl-save">
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

        </div>

        {/* 選中的範本的說明與組成放在固定的底部，不必捲動就看得到。 */}
        <div className="ac-tpl-footer">
          {chosen && (
            <div className="ac-tpl-detail" data-testid="tpl-detail">
              <b>{chosen.name}</b>
              {chosenBuiltin && <p>{chosenBuiltin.description}</p>}
              {chosenRec?.summary && <p>{chosenRec.summary}</p>}
              <div className="ac-tpl-section">{t.ac_tpl_flow}</div>
              <div className="ac-tpl-flow">
                {composition(chosen).map((p, i) => (
                  <span key={`${p.kind}-${p.label}`} className="ac-tpl-flow-step">
                    {i > 0 && (
                      <span className="ac-tpl-flow-arrow" aria-hidden>
                        →
                      </span>
                    )}
                    <span className={`ac-tpl-chip ac-tpl-chip--${p.kind}`}>
                      <KindIcon kind={p.kind} size={13} />
                      {p.label}
                      {p.count > 1 && <em>×{p.count}</em>}
                    </span>
                  </span>
                ))}
              </div>
              {chosenRec && chosenRec.reasons.length > 0 && (
                <>
                  <div className="ac-tpl-section">{t.ac_rec_reasons}</div>
                  <ul className="ac-rec-reasons">
                    {chosenRec.reasons.map((r) => (
                      <li key={r.label}>
                        <b>{r.label}</b>：{r.reason}
                      </li>
                    ))}
                  </ul>
                </>
              )}
              {chosenRec && chosenRec.notes.length > 0 && (
                <p className="ac-rec-notes">
                  {t.ac_rec_notes}：{chosenRec.notes.join("；")}
                </p>
              )}
            </div>
          )}
          {error && <p className="ac-bad-text">{error}</p>}
          <div className="task-dialog-actions">
            {!chosen && <span className="task-field-hint ac-tpl-hint">{t.ac_tpl_pick_hint}</span>}
            <button className="aiterm-btn aiterm-btn--ghost" onClick={onClose}>
              {t.ac_verify_close}
            </button>
            {/* 空桌布：主要是把團隊放上來；已經有團隊：主要是另開一組編制，不要疊在現有的團隊上。 */}
            <button
              className={`aiterm-btn ${agents.length === 0 ? "aiterm-btn--primary" : "aiterm-btn--secondary"}`}
              disabled={!chosen || !repoOk || busy}
              onClick={() => chosen && void apply(chosen)}
            >
              {t.ac_tpl_apply}
            </button>
            {onApplyAsFormation && (
              <button
                className={`aiterm-btn ${agents.length === 0 ? "aiterm-btn--secondary" : "aiterm-btn--primary"}`}
                disabled={!chosen || !repoOk || busy}
                title={t.ac_tpl_apply_new_tip}
                onClick={() => chosen && void apply(chosen, true)}
              >
                {t.ac_tpl_apply_new(chosen?.name ?? "")}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
