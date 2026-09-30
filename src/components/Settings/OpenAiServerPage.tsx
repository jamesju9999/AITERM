import { useCallback, useEffect, useMemo, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { getConfig, type ProviderConfig } from "../../ipc/config";
import {
  openaiServerRegenerateKey,
  openaiServerSetConfig,
  openaiServerStatus,
  type ModelAlias,
  type OpenAiServerConfig,
  type OpenAiServerStatus,
} from "../../ipc/openaiServer";
import "./OpenAiServerPage.css";

/** Tauri 的 `Err(String)` 會以字串 reject；其他情況退回 message 或 JSON。 */
function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}

export function OpenAiServerPage() {
  const { t } = useLocale();
  const [cfg, setCfg] = useState<OpenAiServerConfig | null>(null);
  const [providers, setProviders] = useState<ProviderConfig[]>([]);
  const [status, setStatus] = useState<OpenAiServerStatus | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [keyVisible, setKeyVisible] = useState(false);
  const [confirmRegen, setConfirmRegen] = useState(false);
  const [copied, setCopied] = useState<"key" | "curl" | null>(null);

  useEffect(() => {
    void (async () => {
      const [c, s] = await Promise.all([getConfig(), openaiServerStatus()]);
      setCfg(c.openai_server);
      setProviders(c.providers);
      setStatus(s);
    })();
  }, []);

  const updateCfg = useCallback((next: OpenAiServerConfig) => {
    setSaved(false);
    setCfg(next);
  }, []);

  // 別名重複時後端會拒絕存檔；前端先標紅並禁用儲存，省一次來回。
  const duplicates = useMemo(() => {
    const counts = new Map<string, number>();
    for (const a of cfg?.aliases ?? []) {
      const k = a.alias.trim();
      if (k) counts.set(k, (counts.get(k) ?? 0) + 1);
    }
    return new Set([...counts].filter(([, n]) => n > 1).map(([k]) => k));
  }, [cfg?.aliases]);

  const save = useCallback(async () => {
    if (!cfg) return;
    setSaving(true);
    setError(null);
    try {
      setStatus(await openaiServerSetConfig(cfg));
      setSaved(true);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setSaving(false);
    }
  }, [cfg]);

  const regenerate = useCallback(async () => {
    setError(null);
    try {
      setStatus(await openaiServerRegenerateKey());
      setConfirmRegen(false);
      setCopied(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  const copy = useCallback((what: "key" | "curl", text: string) => {
    void navigator.clipboard.writeText(text);
    setCopied(what);
  }, []);

  if (!cfg) return <div className="openai-server-page" />;

  const setAlias = (i: number, patch: Partial<ModelAlias>) =>
    updateCfg({ ...cfg, aliases: cfg.aliases.map((a, j) => (j === i ? { ...a, ...patch } : a)) });

  const addAlias = () => {
    const first = providers[0];
    updateCfg({
      ...cfg,
      aliases: [...cfg.aliases, { alias: "", provider_id: first?.id ?? "", model: first?.model ?? "" }],
    });
  };

  const host = cfg.allow_lan ? t.openai_server_lan_host_placeholder : "127.0.0.1";
  const port = status?.port ?? cfg.port;
  const baseUrl = `http://${host}:${port}/v1`;
  const key = status?.token ?? null;
  const sampleModel = cfg.aliases.find((a) => a.alias.trim())?.alias ?? "my-model";
  const curl = (token: string) =>
    `curl ${baseUrl}/chat/completions \\\n` +
    `  -H "Authorization: Bearer ${token}" \\\n` +
    `  -H "Content-Type: application/json" \\\n` +
    `  -d '{"model":"${sampleModel}","messages":[{"role":"user","content":"hi"}]}'`;

  return (
    <div className="openai-server-page">
      <h2>{t.openai_server_title}</h2>
      <p className="openai-server-desc">{t.openai_server_desc}</p>

      <div className="openai-server-status">
        <span className={status?.running ? "openai-server-dot openai-server-dot--on" : "openai-server-dot"} />
        {status?.running ? t.openai_server_status_running : t.openai_server_status_stopped}
        {status?.port ? ` · :${status.port}` : ""}
      </div>
      {status?.error && <div className="openai-server-error">{status.error}</div>}
      {error && <div className="openai-server-error" role="alert">{error}</div>}

      <section className="openai-server-section">
        <label className="openai-server-row">
          <input
            type="checkbox"
            checked={cfg.enabled}
            onChange={(e) => updateCfg({ ...cfg, enabled: e.target.checked })}
          />
          {t.openai_server_enable}
        </label>

        <label className="openai-server-row">
          {t.openai_server_port}
          <input
            type="number"
            value={cfg.port}
            onChange={(e) => updateCfg({ ...cfg, port: Number(e.target.value) || 8319 })}
          />
        </label>

        <label className="openai-server-row">
          <input
            type="checkbox"
            checked={cfg.allow_lan}
            onChange={(e) => updateCfg({ ...cfg, allow_lan: e.target.checked })}
          />
          {t.openai_server_allow_lan}
        </label>
        {cfg.allow_lan && <p className="openai-server-warning" role="note">{t.openai_server_lan_warning}</p>}
      </section>

      <section className="openai-server-section">
        <h3>{t.openai_server_api_key}</h3>
        {key ? (
          <div className="openai-server-row">
            <code className="openai-server-key">{keyVisible ? key : "•".repeat(24)}</code>
            <button onClick={() => setKeyVisible((v) => !v)}>
              {keyVisible ? t.openai_server_key_hide : t.openai_server_key_show}
            </button>
            <button onClick={() => copy("key", key)}>
              {copied === "key" ? t.openai_server_key_copied : t.openai_server_key_copy}
            </button>
            {confirmRegen ? (
              <button className="openai-server-danger" onClick={() => void regenerate()}>
                {t.openai_server_key_regenerate_confirm}
              </button>
            ) : (
              <button onClick={() => setConfirmRegen(true)}>{t.openai_server_key_regenerate}</button>
            )}
          </div>
        ) : (
          <p className="openai-server-desc">{t.openai_server_key_pending}</p>
        )}
      </section>

      <section className="openai-server-section">
        <h3>{t.openai_server_aliases}</h3>
        <p className="openai-server-desc">{t.openai_server_aliases_desc}</p>
        {cfg.aliases.map((a, i) => {
          const dup = duplicates.has(a.alias.trim());
          return (
            <div className="openai-server-alias" key={i}>
              <input
                aria-label={t.openai_server_alias_name}
                placeholder={t.openai_server_alias_name}
                className={dup ? "openai-server-invalid" : undefined}
                value={a.alias}
                onChange={(e) => setAlias(i, { alias: e.target.value })}
              />
              <select
                aria-label={t.openai_server_alias_provider}
                value={a.provider_id}
                onChange={(e) => {
                  const p = providers.find((x) => x.id === e.target.value);
                  setAlias(i, { provider_id: e.target.value, model: p?.model ?? a.model });
                }}
              >
                {providers.map((p) => (
                  <option key={p.id} value={p.id}>{p.display_name}</option>
                ))}
              </select>
              <input
                aria-label={t.openai_server_alias_model}
                placeholder={t.openai_server_alias_model}
                value={a.model}
                onChange={(e) => setAlias(i, { model: e.target.value })}
              />
              <button onClick={() => updateCfg({ ...cfg, aliases: cfg.aliases.filter((_, j) => j !== i) })}>
                {t.openai_server_alias_remove}
              </button>
              {dup && <span className="openai-server-error">{t.openai_server_alias_duplicate}</span>}
            </div>
          );
        })}
        <button onClick={addAlias} disabled={providers.length === 0}>{t.openai_server_alias_add}</button>
      </section>

      {status?.running && (
        <section className="openai-server-section">
          <h3>{t.openai_server_section_usage}</h3>
          <div className="openai-server-command">base_url = {baseUrl}</div>
          <div className="openai-server-command">{curl("<API key>")}</div>
        </section>
      )}

      <div className="openai-server-actions">
        <button onClick={() => void save()} disabled={saving || duplicates.size > 0}>
          {saved ? `${t.openai_server_saved} ✓` : t.save}
        </button>
        {status?.running && key && (
          <button onClick={() => copy("curl", curl(key))}>
            {copied === "curl" ? t.openai_server_key_copied : t.openai_server_copy_curl}
          </button>
        )}
      </div>
    </div>
  );
}
