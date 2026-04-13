import { useState, useEffect, useCallback } from "react";
import {
  listProviders,
  addProvider,
  updateProvider,
  removeProvider,
  setDefaultProvider,
  testProvider,
} from "../../ipc/provider";
import type { ProviderInfo, ProviderInput } from "../../ipc/provider";
import { PROVIDER_TYPE_LABELS } from "../../ipc/provider";
import { ProviderForm } from "./ProviderForm";
import "./ProvidersPage.css";

type FormMode = { kind: "add" } | { kind: "edit"; provider: ProviderInfo } | null;

export function ProvidersPage() {
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [formMode, setFormMode] = useState<FormMode>(null);
  const [testing, setTesting] = useState<string | null>(null);
  const [testResults, setTestResults] = useState<Record<string, string>>({});
  const reload = useCallback(async () => {
    const list = await listProviders();
    setProviders(list);
    // is_default is already set on each ProviderInfo by the backend
  }, []);

  useEffect(() => { reload(); }, [reload]);

  const handleAdd = async (input: ProviderInput) => {
    await addProvider(input);
    await reload();
    setFormMode(null);
  };

  const handleEdit = async (input: ProviderInput) => {
    await updateProvider(input);
    await reload();
    setFormMode(null);
  };

  const handleRemove = async (id: string) => {
    if (!confirm(`確定移除 provider「${id}」嗎？`)) return;
    await removeProvider(id);
    await reload();
  };

  const handleSetDefault = async (id: string) => {
    await setDefaultProvider(id);
    await reload();
  };

  const handleTest = async (id: string) => {
    setTesting(id);
    setTestResults((r) => ({ ...r, [id]: "" }));
    try {
      await testProvider(id);
      setTestResults((r) => ({ ...r, [id]: "✓ 連線成功" }));
    } catch (e: unknown) {
      const msg = typeof e === "object" && e !== null && "message" in e
        ? String((e as { message: string }).message)
        : String(e);
      setTestResults((r) => ({ ...r, [id]: `✗ ${msg}` }));
    } finally {
      setTesting(null);
    }
  };

  return (
    <div className="providers-page">
      <div className="providers-header">
        <h2>AI Providers</h2>
        <button
          className="btn-add"
          onClick={() => setFormMode({ kind: "add" })}
        >
          + 新增 Provider
        </button>
      </div>

      {providers.length === 0 && !formMode && (
        <p className="providers-empty">
          尚未設定任何 Provider。點擊「新增 Provider」開始設定。
        </p>
      )}

      <div className="provider-list">
        {providers.map((p) => (
          <div
            key={p.id}
            className={`provider-card ${p.is_default ? "provider-card--default" : ""}`}
          >
            <div className="provider-card-left">
              <div className="provider-name">
                {p.display_name}
                {p.is_default && (
                  <span className="badge-default">預設</span>
                )}
              </div>
              <div className="provider-meta">
                {PROVIDER_TYPE_LABELS[p.provider_type]} · {p.model}
                {p.base_url && (
                  <span className="provider-url"> · {p.base_url}</span>
                )}
              </div>
              {testResults[p.id] && (
                <div
                  className={`test-result ${testResults[p.id].startsWith("✓") ? "test-result--ok" : "test-result--err"}`}
                >
                  {testResults[p.id]}
                </div>
              )}
            </div>

            <div className="provider-card-actions">
              <button
                onClick={() => handleTest(p.id)}
                disabled={testing === p.id}
                title="測試連線"
              >
                {testing === p.id ? "測試中…" : "測試"}
              </button>
              {!p.is_default && (
                <button onClick={() => handleSetDefault(p.id)} title="設為預設">
                  設為預設
                </button>
              )}
              <button
                onClick={() => setFormMode({ kind: "edit", provider: p })}
                title="編輯"
              >
                編輯
              </button>
              <button
                className="btn-danger"
                onClick={() => handleRemove(p.id)}
                title="移除"
              >
                移除
              </button>
            </div>
          </div>
        ))}
      </div>

      {formMode && (
        <div className="provider-form-overlay">
          <div className="provider-form-panel">
            <ProviderForm
              existing={formMode.kind === "edit" ? formMode.provider : undefined}
              onSave={formMode.kind === "add" ? handleAdd : handleEdit}
              onCancel={() => setFormMode(null)}
            />
          </div>
        </div>
      )}
    </div>
  );
}
