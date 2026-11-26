import { useEffect, useState } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import { useLocale } from "../../contexts/LocaleContext";
import { ruleProblem } from "./denyRule";
import { getProjectDeny, resetGlobalDeny, setGlobalDeny, setProjectDeny, type DenyOverride, type DenyRule } from "../../ipc/serverVerify";

type Tab = "global" | "project";

/**
 * 驗收者的禁止清單。兩個分頁：「全域預設」（整台 server 一份）與「這個專案」（只能新增專屬規則，或把全域的某一條在這個專案移除）。
 * 只有管理員看得到入口；這不是沙箱，只擋最明顯的誤操作（說明寫在對話框裡）。
 */
export function DenyListDialog({ projectId, onClose }: { projectId: string; onClose: () => void }) {
  const { t } = useLocale();
  const [tab, setTab] = useState<Tab>("project");
  const [global, setGlobal] = useState<DenyRule[] | null>(null);
  const [overrides, setOverrides] = useState<DenyOverride[] | null>(null);
  const [effective, setEffective] = useState<string[]>([]);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

  useEffect(() => {
    let alive = true;
    getProjectDeny(projectId)
      .then((p) => {
        if (!alive) return;
        setGlobal(p.global);
        setOverrides(p.overrides);
        setEffective(p.effective);
      })
      .catch((e) => alive && setError(errText(e)));
    return () => {
      alive = false;
    };
  }, [projectId]);

  const addRule = () => {
    const rule = draft.trim();
    if (!rule) return;
    if (ruleProblem(rule)) {
      setError(t.ac_deny_bad_rule);
      return;
    }
    setError(null);
    if (tab === "global") {
      if ((global ?? []).some((r) => r.rule === rule)) return;
      setGlobal([...(global ?? []), { rule, note: t.ac_deny_custom_note, enabled: true }]);
    } else {
      if ((overrides ?? []).some((o) => o.rule === rule)) return;
      setOverrides([...(overrides ?? []), { rule, kind: "add", enabled: true }]);
    }
    setDraft("");
  };

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      if (tab === "global" && global) {
        const g = await setGlobalDeny(global);
        setGlobal(g);
      } else if (overrides) {
        const p = await setProjectDeny(projectId, overrides);
        setOverrides(p.overrides);
        setEffective(p.effective);
      }
      // 全域改了，專案實際生效的規則也會變：重抓一次。
      const p = await getProjectDeny(projectId);
      setGlobal(p.global);
      setOverrides(p.overrides);
      setEffective(p.effective);
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const reset = async () => {
    if (!(await confirm(t.ac_deny_reset_confirm))) return;
    setBusy(true);
    try {
      setGlobal(await resetGlobalDeny());
      const p = await getProjectDeny(projectId);
      setOverrides(p.overrides);
      setEffective(p.effective);
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  /** 專案分頁：全域規則可以在這個專案「移除」（再按一次恢復）。 */
  const removedHere = (rule: string) => (overrides ?? []).some((o) => o.kind === "remove" && o.rule === rule && o.enabled);
  const toggleRemoved = (rule: string) => {
    const list = overrides ?? [];
    setOverrides(removedHere(rule) ? list.filter((o) => !(o.kind === "remove" && o.rule === rule)) : [...list.filter((o) => o.rule !== rule), { rule, kind: "remove", enabled: true }]);
  };

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-deny-dialog" role="dialog" aria-label={t.ac_deny_title}>
        <h3 className="task-dialog-title">{t.ac_deny_title}</h3>
        <p className="ac-dialog-desc">{t.ac_deny_desc}</p>
        <div className="ac-deny-tabs" role="tablist">
          <button role="tab" aria-selected={tab === "project"} className={`aiterm-btn aiterm-btn--sm${tab === "project" ? " aiterm-btn--primary" : ""}`} onClick={() => setTab("project")}>
            {t.ac_deny_tab_project}
          </button>
          <button role="tab" aria-selected={tab === "global"} className={`aiterm-btn aiterm-btn--sm${tab === "global" ? " aiterm-btn--primary" : ""}`} onClick={() => setTab("global")}>
            {t.ac_deny_tab_global}
          </button>
        </div>

        {tab === "global" && (
          <ul className="ac-deny-list" data-testid="deny-global">
            {(global ?? []).map((r) => (
              <li key={r.rule}>
                <label>
                  <input type="checkbox" checked={r.enabled} onChange={() => setGlobal((global ?? []).map((x) => (x.rule === r.rule ? { ...x, enabled: !x.enabled } : x)))} />
                  <code>{r.rule}</code>
                  <span className="ac-deny-note">{r.note}</span>
                </label>
                <button className="ac-link" onClick={() => setGlobal((global ?? []).filter((x) => x.rule !== r.rule))}>
                  {t.ac_deny_delete}
                </button>
              </li>
            ))}
          </ul>
        )}

        {tab === "project" && (
          <>
            <p className="ac-dialog-desc">{t.ac_deny_project_desc}</p>
            <ul className="ac-deny-list" data-testid="deny-project">
              {(global ?? []).map((r) => (
                <li key={r.rule} className={r.enabled ? "" : "ac-deny-off"}>
                  <label>
                    <input type="checkbox" checked={r.enabled && !removedHere(r.rule)} disabled={!r.enabled} onChange={() => toggleRemoved(r.rule)} aria-label={r.rule} />
                    <code>{r.rule}</code>
                    <span className="ac-deny-note">{r.enabled ? r.note : t.ac_deny_global_off}</span>
                  </label>
                </li>
              ))}
              {(overrides ?? [])
                .filter((o) => o.kind === "add")
                .map((o) => (
                  <li key={o.rule}>
                    <label>
                      <input type="checkbox" checked={o.enabled} onChange={() => setOverrides((overrides ?? []).map((x) => (x.rule === o.rule ? { ...x, enabled: !x.enabled } : x)))} aria-label={o.rule} />
                      <code>{o.rule}</code>
                      <span className="ac-deny-note">{t.ac_deny_project_only}</span>
                    </label>
                    <button className="ac-link" onClick={() => setOverrides((overrides ?? []).filter((x) => x.rule !== o.rule))}>
                      {t.ac_deny_delete}
                    </button>
                  </li>
                ))}
            </ul>
            <p className="ac-dialog-desc" data-testid="deny-effective">
              {t.ac_deny_effective(effective.length)}
            </p>
          </>
        )}

        <div className="ac-deny-add">
          <input className="task-field-input" aria-label={t.ac_deny_add_label} placeholder="Bash(make deploy:*)" value={draft} onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => e.key === "Enter" && addRule()} />
          <button className="aiterm-btn aiterm-btn--secondary" onClick={addRule}>
            {t.ac_deny_add}
          </button>
        </div>
        {error && (
          <p className="ac-bad-text" role="alert">
            {error}
          </p>
        )}
        <div className="task-dialog-actions">
          {tab === "global" && (
            <button className="aiterm-btn aiterm-btn--secondary" disabled={busy} onClick={() => void reset()}>
              {t.ac_deny_reset}
            </button>
          )}
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.ac_verify_close}
          </button>
          <button className="aiterm-btn aiterm-btn--primary" disabled={busy || (tab === "global" ? !global : !overrides)} onClick={() => void save()}>
            {t.ac_deny_save}
          </button>
        </div>
      </div>
    </div>
  );
}
