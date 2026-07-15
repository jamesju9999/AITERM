import { useState, useEffect } from "react";

const ANTHROPIC_CLAUDE_MODELS = [
  "claude-opus-4-8",
  "claude-sonnet-4-6",
  "claude-haiku-4-5-20251001",
  "claude-3-7-sonnet-20250219",
  "claude-3-5-sonnet-20241022",
  "claude-3-5-haiku-20241022",
  "claude-3-opus-20240229",
];
import type { ProviderInput, ProviderInfo } from "../../ipc/provider";
import {
  PROVIDER_TYPE_LABELS,
  DEFAULT_MODELS,
  DEFAULT_BASE_URLS,
  COMPATIBLE_PRESETS,
  getOllamaModels,
  githubCopilotDeviceStart,
  githubCopilotDevicePoll,
  getGithubCopilotModels,
  getGithubCopilotModelsByProvider,
  getGoogleAiModels,
  getGoogleAiModelsByProvider,
  anthropicOAuthStart,
  anthropicOAuthComplete,
  anthropicOAuthLogout,
  getAnthropicOAuthModels,

} from "../../ipc/provider";
import type { ProviderType } from "../../ipc/config";
import { useLocale } from "../../contexts/LocaleContext";
import "./ProviderForm.css";

interface Props {
  existing?: ProviderInfo;
  onSave: (input: ProviderInput) => Promise<void>;
  onCancel: () => void;
}

const PROVIDER_TYPES: ProviderType[] = [
  "openai",
  "anthropic",
  "ollama",
  "openai-compatible",
  "github-copilot",
  "google-ai",
];

export function ProviderForm({ existing, onSave, onCancel }: Props) {
  const { t } = useLocale();
  const isEdit = !!existing;

  const [id, setId] = useState(existing?.id ?? "");
  const [displayName, setDisplayName] = useState(existing?.display_name ?? "");
  const [providerType, setProviderType] = useState<ProviderType>(
    existing?.provider_type ?? "openai"
  );
  const [baseUrl, setBaseUrl] = useState(existing?.base_url ?? "");
  const [model, setModel] = useState(existing?.model ?? "");
  const [apiKey, setApiKey] = useState("");
  const [oauthClientId, setOauthClientId] = useState(existing?.oauth_client_id ?? "");
  const [supportsJsonMode, setSupportsJsonMode] = useState(
    existing?.supports_json_mode ?? true
  );
  const [anthropicAuthMethod, setAnthropicAuthMethod] = useState<"api_key" | "oauth">(
    existing?.auth_method === "oauth" ? "oauth" : "api_key"
  );
  const [anthropicOAuthLoggedIn, setAnthropicOAuthLoggedIn] = useState(
    existing?.auth_method === "oauth" && (existing?.has_api_key ?? false)
  );
  const [anthropicOAuthCode, setAnthropicOAuthCode] = useState("");
  const [anthropicModels, setAnthropicModels] = useState<string[]>(ANTHROPIC_CLAUDE_MODELS);
  const [anthropicModelsLoading, setAnthropicModelsLoading] = useState(false);
  const [ollamaModels, setOllamaModels] = useState<string[]>([]);
  const [ollamaLoading, setOllamaLoading] = useState(false);
  const [copilotModels, setCopilotModels] = useState<string[]>([]);
  const [copilotLoading, setCopilotLoading] = useState(false);
  const [googleAiModels, setGoogleAiModels] = useState<string[]>([]);
  const [googleAiLoading, setGoogleAiLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [authing, setAuthing] = useState(false);
  const [authStatus, setAuthStatus] = useState<string | null>(null);

  const fetchAnthropicModels = (providerId: string) => {
    setAnthropicModelsLoading(true);
    getAnthropicOAuthModels(providerId)
      .then((models) => { if (models.length > 0) setAnthropicModels(models); })
      .catch(() => {})
      .finally(() => setAnthropicModelsLoading(false));
  };

  useEffect(() => {
    if (isEdit && existing?.auth_method === "oauth" && existing?.has_api_key && existing?.id) {
      if (existing.provider_type === "anthropic") {
        fetchAnthropicModels(existing.id);
      }
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!isEdit) {
      setModel(DEFAULT_MODELS[providerType]);
      setBaseUrl(DEFAULT_BASE_URLS[providerType]);
      setSupportsJsonMode(providerType !== "ollama");
    }
  }, [providerType, isEdit]);

  useEffect(() => {
    if (providerType !== "ollama") return;
    setOllamaLoading(true);
    getOllamaModels(baseUrl || undefined)
      .then((models) => setOllamaModels(models))
      .catch(() => setOllamaModels([]))
      .finally(() => setOllamaLoading(false));
  }, [providerType, baseUrl]);

  const loadCopilotModels = async (token: string) => {
    setCopilotLoading(true);
    try {
      const models = await getGithubCopilotModels(token, baseUrl || undefined);
      setCopilotModels(models);
      if (models.length > 0 && !models.includes(model)) {
        setModel(models[0]);
      }
    } catch {
      setCopilotModels([]);
    } finally {
      setCopilotLoading(false);
    }
  };

  useEffect(() => {
    if (providerType !== "github-copilot") return;
    if (apiKey.trim()) {
      loadCopilotModels(apiKey.trim());
      return;
    }
    if (isEdit && existing?.has_api_key && id.trim()) {
      setCopilotLoading(true);
      getGithubCopilotModelsByProvider(id.trim(), baseUrl || undefined)
        .then((models) => {
          setCopilotModels(models);
          if (models.length > 0 && !models.includes(model)) {
            setModel(models[0]);
          }
          setAuthStatus(t.provider_auth_ok);
        })
        .catch(() => {
          setCopilotModels([]);
        })
        .finally(() => setCopilotLoading(false));
      return;
    }
    setCopilotModels([]);
  }, [providerType, apiKey, baseUrl, isEdit, existing?.has_api_key, id, model, t.provider_auth_ok]);

  useEffect(() => {
    if (providerType !== "google-ai") return;

    // Edit mode with existing saved key — fetch immediately (no debounce needed).
    if (!apiKey.trim() && isEdit && existing?.has_api_key && id.trim()) {
      setGoogleAiLoading(true);
      getGoogleAiModelsByProvider(id.trim())
        .then((models) => {
          setGoogleAiModels(models);
          if (models.length > 0 && !models.includes(model)) setModel(models[0]);
        })
        .catch(() => setGoogleAiModels([]))
        .finally(() => setGoogleAiLoading(false));
      return;
    }

    if (!apiKey.trim()) { setGoogleAiModels([]); return; }

    // Debounce: wait 500 ms after user stops typing before calling the API.
    const timer = setTimeout(() => {
      setGoogleAiLoading(true);
      getGoogleAiModels(apiKey.trim())
        .then((models) => {
          setGoogleAiModels(models);
          if (models.length > 0 && !models.includes(model)) setModel(models[0]);
        })
        .catch(() => setGoogleAiModels([]))
        .finally(() => setGoogleAiLoading(false));
    }, 500);

    return () => clearTimeout(timer);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providerType, apiKey, isEdit, existing?.has_api_key, id]);

  const handleSave = async () => {
    setError(null);
    if (!id.trim()) { setError(t.err_id_empty); return; }
    if (!displayName.trim()) { setError(t.err_name_empty); return; }
    if (!model.trim()) { setError(t.err_model_empty); return; }
    if (
      providerType === "github-copilot" &&
      !apiKey.trim() &&
      !(isEdit && existing?.has_api_key)
    ) {
      setError(t.err_copilot_auth_required);
      return;
    }
    if (providerType === "openai-compatible" && !baseUrl.trim()) {
      setError(t.err_base_url_required);
      return;
    }
    setSaving(true);
    try {
      await onSave({
        id: id.trim(),
        display_name: displayName.trim(),
        provider_type: providerType,
        base_url: baseUrl.trim() || null,
        oauth_client_id: oauthClientId.trim() || null,
        model: model.trim(),
        supports_json_mode: supportsJsonMode,
        api_key: apiKey.trim() || null,
        auth_method: providerType === "anthropic" ? anthropicAuthMethod : null,
      });
    } catch (e: unknown) {
      setError(typeof e === "string" ? e : t.err_save_failed);
    } finally {
      setSaving(false);
    }
  };

  const runCopilotDeviceAuth = async () => {
    setAuthing(true);
    setAuthStatus(null);
    try {
      const started = await githubCopilotDeviceStart(oauthClientId.trim() || undefined);
      setAuthStatus(`${t.provider_auth_pending} ${started.user_code}`);

      let interval = Math.max(1, started.interval);
      const deadline = Date.now() + started.expires_in * 1000;

      while (Date.now() < deadline) {
        await new Promise((resolve) => setTimeout(resolve, interval * 1000));
        const poll = await githubCopilotDevicePoll(started.device_code, oauthClientId.trim() || undefined);

        if (poll.status === "authorized") {
          setApiKey(poll.access_token);
          setAuthStatus(t.provider_auth_ok);
          await loadCopilotModels(poll.access_token);

          // In edit mode, persist token + OAuth client id immediately so
          // the provider-level "Test" action works without requiring a
          // separate manual save click.
          if (isEdit) {
            await onSave({
              id: id.trim(),
              display_name: displayName.trim(),
              provider_type: providerType,
              base_url: baseUrl.trim() || null,
              oauth_client_id: oauthClientId.trim() || null,
              model: model.trim(),
              supports_json_mode: supportsJsonMode,
              api_key: poll.access_token,
              auth_method: null,
            });
          }
          return;
        }

        if (poll.status === "authorization_pending") {
          continue;
        }

        if (poll.status === "slow_down") {
          interval += 5;
          continue;
        }

        if (poll.status === "access_denied" || poll.status === "expired_token" || poll.status === "error") {
          setAuthStatus(poll.message);
          return;
        }
      }

      setAuthStatus(t.provider_auth_timeout);
    } catch (e: unknown) {
      setAuthStatus(String(e));
    } finally {
      setAuthing(false);
    }
  };

  return (
    <div className="provider-form">
      <h3>{isEdit ? t.edit_provider : t.new_provider}</h3>

      {error && <div className="form-error">{error}</div>}

      <div className="form-group">
        <label>{t.provider_type}</label>
        <select
          value={providerType}
          onChange={(e) => setProviderType(e.target.value as ProviderType)}
          disabled={isEdit}
        >
          {PROVIDER_TYPES.map((pt) => (
            <option key={pt} value={pt}>
              {PROVIDER_TYPE_LABELS[pt]}
            </option>
          ))}
        </select>
      </div>

      <div className="form-group">
        <label>ID</label>
        <input
          type="text"
          value={id}
          onChange={(e) => setId(e.target.value)}
          placeholder="e.g. claude-sonnet"
          disabled={isEdit}
        />
      </div>

      <div className="form-group">
        <label>{t.provider_display_name}</label>
        <input
          type="text"
          value={displayName}
          onChange={(e) => setDisplayName(e.target.value)}
          placeholder="e.g. Claude Sonnet"
        />
      </div>

      {providerType === "anthropic" && (
        <div className="form-group">
          <label>{t.settings_provider_auth_type}</label>
          <div className="anthropic-auth-tabs">
            <button
              type="button"
              className={`auth-tab ${anthropicAuthMethod === "api_key" ? "active" : ""}`}
              onClick={() => { setAnthropicAuthMethod("api_key"); setAnthropicOAuthCode(""); setAuthStatus(null); }}
            >
              {t.settings_provider_auth_api_key}
            </button>
            <button
              type="button"
              className={`auth-tab ${anthropicAuthMethod === "oauth" ? "active" : ""}`}
              onClick={() => { setAnthropicAuthMethod("oauth"); setApiKey(""); setAuthStatus(null); }}
            >
              {t.settings_provider_auth_oauth("Claude Pro/Max")}
            </button>
          </div>
        </div>
      )}

      {providerType === "anthropic" && anthropicAuthMethod === "api_key" && (
        <div className="form-group">
          <label>{t.provider_api_key}</label>
          <input
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={isEdit ? t.provider_api_key_placeholder_edit : t.provider_api_key_placeholder_new}
            autoComplete="off"
          />
        </div>
      )}

      {providerType === "anthropic" && anthropicAuthMethod === "oauth" && (
        <div className="form-group">
          <label>{t.settings_provider_auth_oauth("")}</label>
          {anthropicOAuthLoggedIn ? (
            <div className="anthropic-oauth-done">
              <span className="anthropic-oauth-ok">{t.settings_provider_oauth_ok}</span>
              <button
                type="button"
                className="anthropic-oauth-logout"
                disabled={saving}
                onClick={async () => {
                  if (!isEdit) return;
                  try {
                    await anthropicOAuthLogout(id.trim());
                    setAnthropicOAuthLoggedIn(false);
                    setAnthropicAuthMethod("api_key");
                  } catch (e: unknown) {
                    setAuthStatus(String(e));
                  }
                }}
              >
                {t.settings_provider_oauth_logout}
              </button>
            </div>
          ) : (
            <div className="anthropic-oauth-flow">
              {authing ? (
                <div className="anthropic-oauth-paste-ui">
                  <p className="anthropic-oauth-paste-hint">
                    {t.settings_provider_oauth_instructions}
                  </p>
                  <textarea
                    className="anthropic-oauth-url-input"
                    value={anthropicOAuthCode}
                    onChange={(e) => setAnthropicOAuthCode(e.target.value)}
                    placeholder={t.settings_provider_oauth_placeholder}
                    rows={2}
                    autoFocus
                  />
                  <div className="anthropic-oauth-btns">
                    <button
                      type="button"
                      className="anthropic-oauth-open"
                      disabled={!anthropicOAuthCode.trim()}
                      onClick={async () => {
                        const pid = id.trim();
                        if (!pid) {
                          setAuthStatus(t.settings_provider_oauth_err_save_first);
                          setAuthing(false);
                          return;
                        }
                        try {
                          await anthropicOAuthComplete(pid, anthropicOAuthCode.trim());
                          setAnthropicOAuthLoggedIn(true);
                          setAuthStatus(t.settings_provider_oauth_success);
                          setAnthropicOAuthCode("");
                          fetchAnthropicModels(pid);
                        } catch (e: unknown) {
                          setAuthStatus(t.settings_provider_oauth_err(String(e)));
                        } finally {
                          setAuthing(false);
                        }
                      }}
                    >
                      {t.settings_provider_btn_confirm_auth}
                    </button>
                    <button
                      type="button"
                      className="anthropic-oauth-cancel-btn"
                      onClick={() => { setAuthing(false); setAnthropicOAuthCode(""); }}
                    >
                      {t.settings_provider_btn_cancel}
                    </button>
                  </div>
                </div>
              ) : (
                <>
                  <button
                    type="button"
                    className="anthropic-oauth-open"
                    disabled={!id.trim()}
                    onClick={async () => {
                      setAuthing(true);
                      setAuthStatus(null);
                      try {
                        await anthropicOAuthStart();
                      } catch (e: unknown) {
                        setAuthStatus(t.settings_provider_oauth_err(String(e)));
                        setAuthing(false);
                      }
                    }}
                  >
                    {t.settings_provider_btn_open_auth}
                  </button>
                  {!id.trim() && (
                    <div className="form-hint">{t.settings_provider_oauth_id_required}</div>
                  )}
                </>
              )}
              {authStatus && (
                <div className={`form-hint ${authStatus.startsWith("錯誤") || authStatus.startsWith("Error") ? "form-hint--error" : ""}`}>
                  {authStatus}
                </div>
              )}
            </div>
          )}
          {anthropicOAuthLoggedIn && authStatus && <div className="form-hint">{authStatus}</div>}
        </div>
      )}

      {providerType !== "ollama" && providerType !== "github-copilot" && providerType !== "anthropic" && (
        <div className="form-group">
          <label>
            {providerType === "openai-compatible" ? t.provider_api_key_optional : t.provider_api_key}
          </label>
          <input
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={isEdit ? t.provider_api_key_placeholder_edit : t.provider_api_key_placeholder_new}
            autoComplete="off"
          />
        </div>
      )}

      {(providerType === "ollama" ||
        providerType === "openai-compatible" ||
        providerType === "github-copilot" ||
        providerType === "google-ai") && (
        <div className="form-group">
          <label>{t.provider_base_url}</label>
          {providerType === "openai-compatible" && (
            <div className="presets">
              {COMPATIBLE_PRESETS.map((p) => (
                <button
                  key={p.label}
                  type="button"
                  className="preset-btn"
                  onClick={() => setBaseUrl(p.url)}
                >
                  {p.label}
                </button>
              ))}
            </div>
          )}
          <input
            type="text"
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            placeholder={DEFAULT_BASE_URLS[providerType] || "https://..."}
          />
        </div>
      )}

      {providerType === "github-copilot" && (
        <div className="form-group">
          <label>{t.provider_oauth_client_id}</label>
          <input
            type="text"
            value={oauthClientId}
            onChange={(e) => setOauthClientId(e.target.value)}
            placeholder={t.provider_oauth_client_id_placeholder}
          />
          <label>{t.provider_auth_action}</label>
          <button type="button" onClick={runCopilotDeviceAuth} disabled={authing}>
            {authing
              ? t.provider_auth_running
              : (apiKey.trim() || (isEdit && existing?.has_api_key))
                ? t.provider_auth_ok
                : t.provider_copilot_device_auth}
          </button>
          {authStatus && <div className="form-hint">{authStatus}</div>}
        </div>
      )}

      <div className="form-group">
        <label>{t.provider_model}</label>
        {providerType === "ollama" ? (
          ollamaLoading ? (
            <input type="text" value={t.provider_ollama_loading} disabled />
          ) : ollamaModels.length > 0 ? (
            <select value={model} onChange={(e) => setModel(e.target.value)}>
              {ollamaModels.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          ) : (
            <input
              type="text"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder={t.provider_ollama_fallback_placeholder}
            />
          )
        ) : providerType === "github-copilot" ? (
          copilotLoading ? (
            <input type="text" value={t.provider_model_loading} disabled />
          ) : copilotModels.length > 0 ? (
            <select value={model} onChange={(e) => setModel(e.target.value)}>
              {copilotModels.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          ) : (
            <input
              type="text"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder={DEFAULT_MODELS[providerType]}
            />
          )
        ) : providerType === "google-ai" ? (
          googleAiLoading ? (
            <input type="text" value={t.provider_model_loading} disabled />
          ) : (
            <>
              <input
                type="text"
                list="google-ai-models-list"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={googleAiModels.length > 0 ? t.settings_provider_model_placeholder : DEFAULT_MODELS[providerType]}
              />
              {googleAiModels.length > 0 && (
                <datalist id="google-ai-models-list">
                  {googleAiModels.map((m) => (
                    <option key={m} value={m} />
                  ))}
                </datalist>
              )}
            </>
          )
        ) : providerType === "anthropic" && anthropicAuthMethod === "oauth" && anthropicOAuthLoggedIn ? (
          anthropicModelsLoading ? (
            <input type="text" value={t.settings_provider_model_loading_placeholder} disabled />
          ) : (
            <select value={model} onChange={(e) => setModel(e.target.value)}>
              {!anthropicModels.includes(model) && model && (
                <option value={model}>{model}</option>
              )}
              {anthropicModels.map((m) => (
                <option key={m} value={m}>{m}</option>
              ))}
            </select>
          )
        ) : (
          <input
            type="text"
            value={model}
            onChange={(e) => setModel(e.target.value)}
            placeholder={DEFAULT_MODELS[providerType]}
          />
        )}
      </div>

      {(providerType === "openai-compatible" ||
        providerType === "github-copilot" ||
        providerType === "google-ai") && (
        <div className="form-group form-group--checkbox">
          <label>
            <input
              type="checkbox"
              checked={supportsJsonMode}
              onChange={(e) => setSupportsJsonMode(e.target.checked)}
            />
            {t.provider_json_mode}
          </label>
        </div>
      )}

      <div className="form-actions">
        <button type="button" onClick={onCancel} disabled={saving}>
          {t.cancel}
        </button>
        <button
          type="button"
          className="btn-primary"
          onClick={handleSave}
          disabled={saving}
        >
          {saving ? t.saving_btn : t.save}
        </button>
      </div>
    </div>
  );
}
