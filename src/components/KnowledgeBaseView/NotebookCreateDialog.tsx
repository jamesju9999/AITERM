import { useEffect, useState } from "react";
import { pickFolder } from "../../ipc/vcs";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { useLocale } from "../../contexts/LocaleContext";

interface Props {
  onCreate: (name: string, folderPath: string, embedProviderId?: string, embedModel?: string) => Promise<void>;
  onClose: () => void;
}

// Anthropic 沒有 embedding API，只有這三種 provider 類型可用於 embedding。
const EMBEDDING_CAPABLE_TYPES = new Set(["ollama", "openai", "openai-compatible"]);

export function NotebookCreateDialog({ onCreate, onClose }: Props) {
  const { t } = useLocale();
  const [name, setName] = useState("");
  const [folderPath, setFolderPath] = useState("");
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState("");
  const [model, setModel] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listProviders().then((list) => {
      const embeddable = list.filter((p) => EMBEDDING_CAPABLE_TYPES.has(p.provider_type));
      setProviders(embeddable);
      if (embeddable.length > 0) setProviderId(embeddable[0].id);
    }).catch(() => {});
  }, []);

  const handlePickFolder = async () => {
    const folder = await pickFolder();
    if (folder) setFolderPath(folder);
  };

  const handleSubmit = async () => {
    if (!name.trim() || !folderPath || !providerId || !model.trim() || submitting) return;
    setSubmitting(true);
    setError(null);
    try {
      await onCreate(name.trim(), folderPath, providerId, model.trim());
      onClose();
    } catch (e) {
      setError(String(e));
      setSubmitting(false);
    }
  };

  return (
    <div className="kb-dialog-overlay" onClick={onClose}>
      <div className="kb-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="kb-dialog__title">{t.kb_create_dialog_title}</h3>

        <label className="kb-dialog__field">
          <span>{t.kb_create_name_label}</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t.kb_create_name_placeholder}
          />
        </label>

        <label className="kb-dialog__field">
          <span>{t.kb_create_folder_label}</span>
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={handlePickFolder}>
            {folderPath || t.ca_pick_folder}
          </button>
        </label>

        {providers.length === 0 ? (
          <div className="kb-dialog__warning">{t.kb_create_no_provider}</div>
        ) : (
          <>
            <label className="kb-dialog__field">
              <span>{t.kb_create_provider_label}</span>
              <select value={providerId} onChange={(e) => setProviderId(e.target.value)}>
                {providers.map((p) => (
                  <option key={p.id} value={p.id}>{p.display_name}</option>
                ))}
              </select>
            </label>

            <label className="kb-dialog__field">
              <span>{t.kb_create_model_label}</span>
              <input
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={t.kb_create_model_placeholder}
              />
            </label>
          </>
        )}

        {error && <div className="kb-dialog__error">{error}</div>}

        <div className="kb-dialog__actions">
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onClose}>
            {t.ca_cancel}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            onClick={handleSubmit}
            disabled={!name.trim() || !folderPath || !providerId || !model.trim() || submitting}
          >
            {t.kb_create_submit}
          </button>
        </div>
      </div>
    </div>
  );
}
