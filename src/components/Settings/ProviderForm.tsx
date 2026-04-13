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
import "./ProviderForm.css";

interface Props {
  /** Pre-filled when editing an existing provider. */
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

  // When provider type changes, reset defaults.
  useEffect(() => {
    if (!isEdit) {
      setModel(DEFAULT_MODELS[providerType]);
      setBaseUrl(DEFAULT_BASE_URLS[providerType]);
      setSupportsJsonMode(providerType !== "ollama");
    }
  }, [providerType, isEdit]);

  // Load Ollama models when type is ollama.
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
    if (!id.trim()) { setError("ID 不可為空"); return; }
    if (!displayName.trim()) { setError("名稱不可為空"); return; }
    if (!model.trim()) { setError("Model 不可為空"); return; }
    if (providerType === "openai-compatible" && !baseUrl.trim()) {
      setError("OpenAI-Compatible 需要填入 Base URL");
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
      setError(typeof e === "string" ? e : "儲存失敗，請重試");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="provider-form">
      <h3>{isEdit ? "編輯 Provider" : "新增 Provider"}</h3>

      {error && <div className="form-error">{error}</div>}

      {/* Provider type */}
      <div className="form-group">
        <label>類型</label>
        <select
          value={providerType}
          onChange={(e) => setProviderType(e.target.value as ProviderType)}
          disabled={isEdit}
        >
          {PROVIDER_TYPES.map((t) => (
            <option key={t} value={t}>
              {PROVIDER_TYPE_LABELS[t]}
            </option>
          ))}
        </select>
      </div>

      {/* ID (readonly when editing) */}
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

      {/* Display name */}
      <div className="form-group">
        <label>顯示名稱</label>
        <input
          type="text"
          value={displayName}
          onChange={(e) => setDisplayName(e.target.value)}
          placeholder="e.g. Claude Sonnet"
        />
      </div>

      {/* API Key — not shown for Ollama */}
      {providerType !== "ollama" && (
        <div className="form-group">
          <label>
            API Key{providerType === "openai-compatible" ? " (選填)" : ""}
          </label>
          <input
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder={isEdit ? "留空則不更新" : "貼上你的 API Key"}
            autoComplete="off"
          />
        </div>
      )}

      {/* Base URL — for Ollama and Compatible */}
      {(providerType === "ollama" || providerType === "openai-compatible") && (
        <div className="form-group">
          <label>Base URL</label>
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

      {/* Model */}
      <div className="form-group">
        <label>Model</label>
        {providerType === "ollama" ? (
          ollamaLoading ? (
            <input type="text" value="載入中..." disabled />
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
              placeholder="llama3.1:8b（Ollama 未連線，手動輸入）"
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

      {/* JSON mode toggle — only for compatible */}
      {providerType === "openai-compatible" && (
        <div className="form-group form-group--checkbox">
          <label>
            <input
              type="checkbox"
              checked={supportsJsonMode}
              onChange={(e) => setSupportsJsonMode(e.target.checked)}
            />
            支援 JSON Mode（response_format: json_object）
          </label>
        </div>
      )}

      {/* Actions */}
      <div className="form-actions">
        <button type="button" onClick={onCancel} disabled={saving}>
          取消
        </button>
        <button
          type="button"
          className="btn-primary"
          onClick={handleSave}
          disabled={saving}
        >
          {saving ? "儲存中…" : "儲存"}
        </button>
      </div>
    </div>
  );
}
