import { useCallback, useEffect, useState } from "react";
import { intakeProjects, intakeTabCd, intakeTabStart, intakeTabStop, intakeTabs, type IntakeProject } from "../../ipc/intake";
import { getSessionCwd } from "../../ipc/fs";
import { useIntakeChanged } from "../../hooks/useIntakeChanged";
import { useLocale } from "../../contexts/LocaleContext";
import { isUnder, projectForCwd } from "../../lib/intakeProject";
import { MailIcon } from "../Icons";
import "./index.css";

interface Props {
  /** **PTY session id，不是 React 的分頁 id**（後端靠它找到終端機）。 */
  sessionId: string;
}

/** 終端機工具列上的「啟動問題收集」切換鈕：這個分頁成為某個專案問題的接收端。 */
export function IntakeToggle({ sessionId }: Props) {
  const { t } = useLocale();
  const [projectId, setProjectId] = useState<string | null>(null); // 目前收集的專案（null＝沒在收集）
  const [ready, setReady] = useState(true); // 偵測到 Claude Code 在這個分頁裡跑
  const [names, setNames] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [cdError, setCdError] = useState<string | null>(null);
  const [picking, setPicking] = useState<{ projects: IntakeProject[]; choice: string; cwd: string | null } | null>(null);

  const refresh = useCallback(() => {
    Promise.all([intakeTabs(), intakeProjects()])
      .then(([tabs, view]) => {
        const mine = tabs.find((x) => x.tab_id === sessionId);
        setProjectId(mine?.project_id ?? null);
        setReady(mine?.claude_ready ?? true);
        setNames(Object.fromEntries(view.projects.map((p) => [p.id, p.name])));
      })
      .catch(() => {});
  }, [sessionId]);

  useEffect(refresh, [refresh]);
  useIntakeChanged(refresh);

  async function start(project: string) {
    setBusy(true);
    try {
      await intakeTabStart(sessionId, project);
      setPicking(null);
      refresh();
    } finally {
      setBusy(false);
    }
  }

  /** 替這個分頁切換到所選專案的目錄，成功後重新判斷分頁目錄，黃字就會自己消失。 */
  async function goToProjectDir(projectId: string) {
    setBusy(true);
    setCdError(null);
    try {
      await intakeTabCd(sessionId, projectId);
      const cwd = await getSessionCwd(sessionId).catch(() => null);
      setPicking((p) => (p ? { ...p, cwd } : p));
    } catch (e) {
      setCdError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onClick() {
    if (busy) return;
    if (projectId !== null) {
      setBusy(true);
      try {
        await intakeTabStop(sessionId);
        refresh();
      } finally {
        setBusy(false);
      }
      return;
    }
    if (picking) {
      setPicking(null);
      return;
    }
    const [view, cwd] = await Promise.all([intakeProjects(), getSessionCwd(sessionId).catch(() => null)]);
    if (view.projects.length === 0) {
      setPicking({ projects: [], choice: "", cwd });
      return;
    }
    // 只有一個專案就直接開始；多個時預設選「分頁目錄所在的專案」。
    if (view.projects.length === 1) {
      await start(view.projects[0].id);
      return;
    }
    setPicking({ projects: view.projects, choice: (projectForCwd(view.projects, cwd) ?? view.projects[0]).id, cwd });
  }

  const on = projectId !== null;
  const chosen = picking?.projects.find((p) => p.id === picking.choice);
  const outside = !!(picking?.cwd && chosen?.repo_dir && !isUnder(picking.cwd, chosen.repo_dir));

  return (
    <div className="aiterm-intake-toggle">
      <button
        className={`aiterm-btn aiterm-btn--secondary aiterm-btn--sm ${on ? "aiterm-intake-toggle--on" : ""} ${on && !ready ? "aiterm-intake-toggle--waiting" : ""}`}
        title={on && !ready ? t.intake_toggle_waiting_hint : t.intake_toggle_tooltip}
        aria-pressed={on}
        disabled={busy}
        onClick={(e) => {
          e.stopPropagation();
          void onClick();
        }}
        style={{ display: "flex", alignItems: "center", gap: "6px" }}
      >
        <MailIcon size={14} />
        <span>{on ? `${t.intake_toggle_on}${names[projectId] ? `：${names[projectId]}` : ""}${ready ? "" : `（${t.intake_toggle_waiting}）`}` : t.intake_toggle_off}</span>
      </button>
      {picking && (
        <div className="aiterm-intake-toggle__pop" role="dialog" onClick={(e) => e.stopPropagation()}>
          {picking.projects.length === 0 ? (
            <p>{t.intake_no_projects}</p>
          ) : (
            <>
              <label>
                {t.intake_toggle_pick}
                <select value={picking.choice} onChange={(e) => {
                    setCdError(null);
                    setPicking({ ...picking, choice: e.target.value });
                  }}>
                  {picking.projects.map((p) => (
                    <option key={p.id} value={p.id}>{p.name}</option>
                  ))}
                </select>
              </label>
              {outside && (
                <>
                  <p className="aiterm-intake-toggle__warn">{t.intake_toggle_outside.replace("{dir}", chosen?.repo_dir ?? "")}</p>
                  <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={() => void goToProjectDir(picking.choice)}>
                    {t.intake_toggle_cd}
                  </button>
                  {cdError && <p className="aiterm-intake-toggle__err" role="alert">{cdError}</p>}
                </>
              )}
              <div className="aiterm-intake-toggle__row">
                <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={busy} onClick={() => void start(picking.choice)}>
                  {t.intake_toggle_start}
                </button>
                <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => setPicking(null)}>
                  {t.intake_cancel}
                </button>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
