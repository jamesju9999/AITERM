import { confirm } from "@tauri-apps/plugin-dialog";
import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { SystemConnection, SystemLoginInfo, SystemMode, SystemRule } from "../../ipc/systems";
import { restoreWebviewFocus } from "../../lib/webviewFocus";
import { ModelSelect } from "./ModelSelect";
import { hostOf, slug } from "./utils";

const MODES: SystemMode[] = ["read_only", "approve_each", "rules", "autonomous"];
const METHODS = ["*", "POST", "PUT", "PATCH", "DELETE"];

interface Props {
  initial?: SystemConnection;
  /** 已存的自動登入帳號（編輯時）。密碼不會帶回來。 */
  initialLogin?: SystemLoginInfo;
  onSave: (conn: SystemConnection, login: { username: string; password: string }) => void | Promise<void>;
  onCancel: () => void;
}

export function ConnectionForm({ initial, initialLogin, onSave, onCancel }: Props) {
  const { t } = useLocale();
  const [name, setName] = useState(initial?.name ?? "");
  const [url, setUrl] = useState(initial?.start_url ?? "");
  const [hosts, setHosts] = useState((initial?.allowed_hosts ?? []).join(", "));
  const [mode, setMode] = useState<SystemMode>(initial?.mode ?? "read_only");
  const [model, setModel] = useState<string | null>(initial?.model ?? null);
  const [rules, setRules] = useState<SystemRule[]>(initial?.rules ?? []);
  const [username, setUsername] = useState(initialLogin?.username ?? "");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);

  const urlOk = /^https?:\/\//i.test(url.trim()) && hostOf(url) !== "";
  const urlTouched = url.trim() !== "" && url.trim() !== "https://" && url.trim() !== "http://";
  const host = hostOf(url);

  const modeInfo: Record<SystemMode, { title: string; desc: string; risk: string; tone: string }> = {
    read_only: { title: t.sys_mode_title_read_only, desc: t.sys_mode_desc_read_only, risk: t.sys_risk_safe, tone: "safe" },
    approve_each: { title: t.sys_mode_title_approve_each, desc: t.sys_mode_desc_approve_each, risk: t.sys_risk_attended, tone: "info" },
    rules: { title: t.sys_mode_title_rules, desc: t.sys_mode_desc_rules, risk: t.sys_risk_rules, tone: "warn" },
    autonomous: { title: t.sys_mode_title_autonomous, desc: t.sys_mode_desc_autonomous, risk: t.sys_risk_high, tone: "danger" },
  };

  async function submit() {
    // 允許網域留空就帶入起始網址的網域，最常見的情況不用手填。
    const list = hosts.split(",").map((h) => h.trim()).filter(Boolean);
    const allowed = list.length > 0 ? list : [host].filter(Boolean);
    if (mode === "autonomous" && initial?.mode !== "autonomous") {
      const ok = await confirm(t.sys_autonomous_confirm, { kind: "warning" });
      await restoreWebviewFocus();
      if (!ok) return;
    }
    try {
      await onSave({
        id: initial?.id ?? slug(name),
        name: name.trim(),
        start_url: url.trim(),
        allowed_hosts: allowed,
        mode,
        rules: mode === "rules" ? rules.filter((r) => r.url_pattern.trim()) : [],
        enabled: initial?.enabled ?? true,
        model,
      }, { username: username.trim(), password });
    } catch (e) {
      setError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    }
  }

  const setRule = (i: number, patch: Partial<SystemRule>) =>
    setRules((rs) => rs.map((r, j) => (j === i ? { ...r, ...patch } : r)));

  return (
    <form
      className="systems__form"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <label>
        {t.sys_name}
        <input value={name} placeholder={t.sys_name_ph} onChange={(e) => setName(e.target.value)} />
      </label>
      <label>
        {t.sys_start_url}
        <input value={url} placeholder={t.sys_url_ph} onChange={(e) => setUrl(e.target.value)} />
        {urlTouched && !urlOk && <small className="systems__field-error">{t.sys_url_invalid}</small>}
        {urlOk && <small>{t.sys_hosts_default(host)}</small>}
      </label>

      <div className="systems__field-block">
        <div className="systems__field-label">{t.sys_login_title}</div>
        <div className="systems__logingrid">
          <input aria-label={t.sys_login_user} placeholder={t.sys_login_user} autoComplete="off" value={username} onChange={(e) => setUsername(e.target.value)} />
          <input
            type="password"
            aria-label={t.sys_login_pass}
            placeholder={initialLogin?.has_password ? t.sys_login_pass_saved : t.sys_login_pass}
            autoComplete="new-password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </div>
        <small>{t.sys_autologin_hint}</small>
      </div>

      <div className="systems__field-block">
        <div className="systems__field-label" id="sys-mode-label">
          {t.sys_mode}
        </div>
        <div className="systems__modes" role="radiogroup" aria-labelledby="sys-mode-label">
          {MODES.map((m) => (
            <label key={m} className={`systems__mode systems__mode--${modeInfo[m].tone}${mode === m ? " is-on" : ""}`}>
              <input type="radio" name="sys-mode" value={m} checked={mode === m} onChange={() => setMode(m)} />
              <span className="systems__mode-head">
                <strong>{modeInfo[m].title}</strong>
                <span className={`systems__risk systems__risk--${modeInfo[m].tone}`}>{modeInfo[m].risk}</span>
              </span>
              <span className="systems__mode-desc">{modeInfo[m].desc}</span>
            </label>
          ))}
        </div>
      </div>

      {mode === "rules" && (
        <fieldset className="systems__rules">
          <legend>{t.sys_rules}</legend>
          <p className="systems__muted">{t.sys_rules_help}</p>
          {rules.map((r, i) => (
            <div key={i} className="systems__rule" data-testid="rule-row">
              <select aria-label={t.sys_rule_method} value={r.method} onChange={(e) => setRule(i, { method: e.target.value })}>
                {METHODS.map((m) => (
                  <option key={m} value={m}>
                    {m === "*" ? t.sys_rule_method_any : m}
                  </option>
                ))}
              </select>
              <input aria-label={t.sys_rule_pattern} value={r.url_pattern} placeholder={t.sys_rule_pattern_ph} onChange={(e) => setRule(i, { url_pattern: e.target.value })} />
              <select aria-label="action" value={r.action} onChange={(e) => setRule(i, { action: e.target.value as SystemRule["action"] })}>
                <option value="allow">{t.sys_rule_allow}</option>
                <option value="ask">{t.sys_rule_ask}</option>
              </select>
              <button type="button" className="systems__link" aria-label={t.sys_rule_remove} title={t.sys_rule_remove} onClick={() => setRules((rs) => rs.filter((_, j) => j !== i))}>
                ✕
              </button>
            </div>
          ))}
          <div>
            <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => setRules((rs) => [...rs, { method: "POST", url_pattern: "", action: "allow" }])}>
              {t.sys_rule_add}
            </button>
          </div>
        </fieldset>
      )}

      <label>
        {t.sys_model}
        <ModelSelect value={model} onChange={setModel} inheritLabel={t.sys_model_inherit_claude} />
        <small>{t.sys_model_system_default_hint}</small>
      </label>

      <details className="systems__advanced" open={Boolean(initial && hosts)}>
        <summary>{t.sys_hosts_advanced}</summary>
        <label>
          {t.sys_allowed_hosts}
          <input value={hosts} placeholder={host} onChange={(e) => setHosts(e.target.value)} />
          <small>{t.sys_hosts_hint2}</small>
        </label>
      </details>

      {error && (
        <p className="systems__error" role="alert">
          {t.sys_error(error)}
        </p>
      )}
      <div className="systems__actions">
        <button type="submit" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={!name.trim() || !urlOk}>
          {t.sys_save}
        </button>
        <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onCancel}>
          {t.sys_cancel}
        </button>
      </div>
    </form>
  );
}
