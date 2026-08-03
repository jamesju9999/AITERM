import { useEffect, useState } from "react";
import { pickFolder } from "../../ipc/vcs";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { kbListEmbeddingModels } from "../../ipc/knowledgeBase";
import { useLocale } from "../../contexts/LocaleContext";

interface Props {
  onCreate: (name: string, folderPath: string, embedProviderId?: string, embedModel?: string) => Promise<void>;
  onClose: () => void;
}

// Anthropic 沒有 embedding API，只有這三種 provider 類型可用於 embedding。
const EMBEDDING_CAPABLE_TYPES = new Set(["ollama", "openai", "openai-compatible"]);

// base_url 為 null 時 resolve_embedder_config 會套用這些預設值，
// 選單必須顯示同樣的位址，否則畫面上會出現空白的 endpoint。
const DEFAULT_ENDPOINTS: Record<string, string> = {
  ollama: "http://localhost:11434",
  openai: "https://api.openai.com/v1",
};

const TYPE_LABELS: Record<string, string> = {
  ollama: "Ollama",
  openai: "OpenAI",
  "openai-compatible": "OpenAI-Compatible",
};

// 只影響排序，不影響可選性——名稱不含這些字的 embedding 模型仍在清單裡，
// 只是排得比較後面，而使用者永遠可以直接手打。
const EMBEDDING_NAME_HINTS = ["embed", "bge", "gte", "e5", "nomic", "minilm", "mxbai", "jina"];

function looksLikeEmbeddingModel(name: string): boolean {
  const lower = name.toLowerCase();
  return EMBEDDING_NAME_HINTS.some((hint) => lower.includes(hint));
}

function sortEmbeddingFirst(models: string[]): string[] {
  const likely = models.filter(looksLikeEmbeddingModel);
  const rest = models.filter((m) => !looksLikeEmbeddingModel(m));
  return [...likely, ...rest];
}

function providerOptionLabel(p: ProviderInfo): string {
  const endpoint = p.base_url ?? DEFAULT_ENDPOINTS[p.provider_type] ?? "";
  const host = endpoint.replace(/^https?:\/\//, "").replace(/\/+$/, "");
  const type = TYPE_LABELS[p.provider_type] ?? p.provider_type;
  return host ? `${p.display_name} — ${type} · ${host}` : `${p.display_name} — ${type}`;
}

export function NotebookCreateDialog({ onCreate, onClose }: Props) {
  const { t } = useLocale();
  const [name, setName] = useState("");
  const [folderPath, setFolderPath] = useState("");
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState("");
  const [model, setModel] = useState("");
  const [models, setModels] = useState<string[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listProviders().then((list) => {
      const embeddable = list.filter((p) => EMBEDDING_CAPABLE_TYPES.has(p.provider_type));
      setProviders(embeddable);
      if (embeddable.length > 0) setProviderId(embeddable[0].id);
    }).catch(() => {});
  }, []);

  useEffect(() => {
    // providerId 只會從 "" 變成某個真實 id（select 永遠帶值），不會再變回空字串，
    // 所以這裡直接跳過就好；在 effect 本體同步 setModels([]) 只會多一輪 render。
    if (!providerId) return;
    let cancelled = false;
    setModelsLoading(true);
    kbListEmbeddingModels(providerId)
      .then((list) => {
        if (!cancelled) setModels(sortEmbeddingFirst(list));
      })
      .catch(() => {
        // 列舉失敗不擋人：不少自架端點沒有 /v1/models，跳錯誤只是噪音。
        // 靜默退回純文字輸入，使用者仍可手打，建立時的探測會把關。
        if (!cancelled) setModels([]);
      })
      .finally(() => {
        if (!cancelled) setModelsLoading(false);
      });
    return () => { cancelled = true; };
  }, [providerId]);

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
                  <option key={p.id} value={p.id}>{providerOptionLabel(p)}</option>
                ))}
              </select>
            </label>

            <label className="kb-dialog__field">
              <span>{t.kb_create_model_label}</span>
              {modelsLoading ? (
                <input type="text" value={t.provider_model_loading} disabled readOnly />
              ) : (
                <>
                  <input
                    value={model}
                    onChange={(e) => setModel(e.target.value)}
                    placeholder={t.kb_create_model_placeholder}
                    list="kb-embedding-models-list"
                  />
                  <datalist id="kb-embedding-models-list">
                    {models.map((m) => <option key={m} value={m} />)}
                  </datalist>
                </>
              )}
              {model.trim() && !looksLikeEmbeddingModel(model) && (
                <div className="kb-dialog__warning">{t.kb_create_model_unusual}</div>
              )}
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
