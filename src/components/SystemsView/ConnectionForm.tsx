import { confirm } from "@tauri-apps/plugin-dialog";
import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { SystemConnection, SystemMode, SystemRule } from "../../ipc/systems";
import { restoreWebviewFocus } from "../../lib/webviewFocus";
import { hostOf, slug } from "./utils";

const MODES: SystemMode[] = ["read_only", "approve_each", "rules", "autonomous"];

interface Props {
  initial?: SystemConnection;
  onSave: (conn: SystemConnection) => void | Promise<void>;
  onCancel: () => void;
}

export function ConnectionForm({ initial, onSave, onCancel }: Props) {
  const { t } = useLocale();
  const [name, setName] = useState(initial?.name ?? "");
  const [url, setUrl] = useState(initial?.start_url ?? "");
  const [hosts, setHosts] = useState((initial?.allowed_hosts ?? []).join(", "));
  const [mode, setMode] = useState<SystemMode>(initial?.mode ?? "read_only");
  const [rules, setRules] = useState<SystemRule[]>(initial?.rules ?? []);
  const [error, setError] = useState<string | null>(null);

  const modeLabel: Record<SystemMode, string> = {
    read_only: t.sys_mode_read_only,
    approve_each: t.sys_mode_approve_each,
    rules: t.sys_mode_rules,
    autonomous: t.sys_mode_autonomous,
  };

  async function submit() {
    // 允許網域留空就帶入起始網址的網域，最常見的情況不用手填。
    const list = hosts.split(",").map((h) => h.trim()).filter(Boolean);
    const allowed = list.length > 0 ? list : [hostOf(url)].filter(Boolean);
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
      });
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
        <input value={name} onChange={(e) => setName(e.target.value)} />
      </label>
      <label>
        {t.sys_start_url}
        <input value={url} placeholder="https://" onChange={(e) => setUrl(e.target.value)} />
      </label>
      <label>
        {t.sys_allowed_hosts}
        <input value={hosts} placeholder={hostOf(url)} onChange={(e) => setHosts(e.target.value)} />
        <small>{t.sys_allowed_hosts_hint}</small>
      </label>
      <label>
        {t.sys_mode}
        <select value={mode} onChange={(e) => setMode(e.target.value as SystemMode)}>
          {MODES.map((m) => (
            <option key={m} value={m}>
              {modeLabel[m]}
            </option>
          ))}
        </select>
      </label>
      {mode === "rules" && (
        <fieldset className="systems__rules">
          <legend>{t.sys_rules}</legend>
          {rules.map((r, i) => (
            <div key={i} className="systems__rule" data-testid="rule-row">
              <input aria-label={t.sys_rule_method} value={r.method} onChange={(e) => setRule(i, { method: e.target.value })} />
              <input aria-label={t.sys_rule_pattern} value={r.url_pattern} onChange={(e) => setRule(i, { url_pattern: e.target.value })} />
              <select aria-label="action" value={r.action} onChange={(e) => setRule(i, { action: e.target.value as SystemRule["action"] })}>
                <option value="allow">{t.sys_rule_allow}</option>
                <option value="ask">{t.sys_rule_ask}</option>
              </select>
            </div>
          ))}
          <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => setRules((rs) => [...rs, { method: "*", url_pattern: "", action: "ask" }])}>
            {t.sys_rule_add}
          </button>
        </fieldset>
      )}
      {error && (
        <p className="systems__error" role="alert">
          {t.sys_error(error)}
        </p>
      )}
      <div className="systems__actions">
        <button type="submit" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={!name.trim() || !url.trim()}>
          {t.sys_save}
        </button>
        <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onCancel}>
          {t.sys_cancel}
        </button>
      </div>
    </form>
  );
}
