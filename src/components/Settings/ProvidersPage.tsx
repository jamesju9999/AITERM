import { useState, useEffect, useCallback } from "react";
import { formatAiError } from "../../ipc/ai";
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
import { useLocale } from "../../contexts/LocaleContext";
import "./ProvidersPage.css";

type FormMode = { kind: "add" } | { kind: "edit"; provider: ProviderInfo } | null;

export function ProvidersPage() {
  const { t } = useLocale();
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [formMode, setFormMode] = useState<FormMode>(null);
  const [testing, setTesting] = useState<string | null>(null);
  const [testResults, setTestResults] = useState<Record<string, string>>({});
  const [confirmingRemove, setConfirmingRemove] = useState<string | null>(null);
  const reload = useCallback(async () => {
    const list = await listProviders();
    setProviders(list);
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
    await removeProvider(id);
    await reload();
    setConfirmingRemove(null);
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
      setTestResults((r) => ({ ...r, [id]: t.provider_test_ok }));
    } catch (e: unknown) {
      const msg = typeof e === "object" && e !== null && "kind" in e
        ? formatAiError(e as Parameters<typeof formatAiError>[0])
        : typeof e === "object" && e !== null && "message" in e
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
        <h2>{t.ai_providers}</h2>
        <button
          className="btn-add"
          onClick={() => setFormMode({ kind: "add" })}
        >
          {t.add_provider}
        </button>
      </div>

      {providers.length === 0 && !formMode && (
        <p className="providers-empty">{t.no_providers}</p>
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
                  <span className="badge-default">{t.provider_default_badge}</span>
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
                title={t.provider_test}
              >
                {testing === p.id ? t.provider_testing : t.provider_test}
              </button>
              {!p.is_default && (
                <button onClick={() => handleSetDefault(p.id)} title={t.provider_set_default}>
                  {t.provider_set_default}
                </button>
              )}
              <button
                onClick={() => setFormMode({ kind: "edit", provider: p })}
                title={t.edit}
              >
                {t.edit}
              </button>
              {confirmingRemove === p.id ? (
                <>
                  <button onClick={() => setConfirmingRemove(null)}>{t.cancel}</button>
                  <button
                    className="btn-danger"
                    onClick={() => handleRemove(p.id)}
                  >
                    {t.provider_remove}?
                  </button>
                </>
              ) : (
                <button
                  className="btn-danger"
                  onClick={() => setConfirmingRemove(p.id)}
                  title={t.provider_remove}
                >
                  {t.provider_remove}
                </button>
              )}
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
