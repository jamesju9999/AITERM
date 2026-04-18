import { useState, useEffect } from "react";
import type { ProviderInput, ProviderInfo } from "../../ipc/provider";
import {
  PROVIDER_TYPE_LABELS,
  DEFAULT_MODELS,
  DEFAULT_BASE_URLS,
  COMPATIBLE_PRESETS,
  getOllamaModels,
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
  const [supportsJsonMode, setSupportsJsonMode] = useState(
    existing?.supports_json_mode ?? true
  );
  const [ollamaModels, setOllamaModels] = useState<string[]>([]);
  const [ollamaLoading, setOllamaLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  const handleSave = async () => {
    setError(null);
    if (!id.trim()) { setError(t.err_id_empty); return; }
    if (!displayName.trim()) { setError(t.err_name_empty); return; }
    if (!model.trim()) { setError(t.err_model_empty); return; }
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
        model: model.trim(),
        supports_json_mode: supportsJsonMode,
        api_key: apiKey.trim() || null,
      });
    } catch (e: unknown) {
      setError(typeof e === "string" ? e : t.err_save_failed);
    } finally {
      setSaving(false);
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

      {providerType !== "ollama" && (
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

      {(providerType === "ollama" || providerType === "openai-compatible") && (
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
        ) : (
          <input
            type="text"
            value={model}
            onChange={(e) => setModel(e.target.value)}
            placeholder={DEFAULT_MODELS[providerType]}
          />
        )}
      </div>

      {providerType === "openai-compatible" && (
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
