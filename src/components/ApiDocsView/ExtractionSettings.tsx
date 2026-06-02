// src/components/ApiDocsView/ExtractionSettings.tsx
import { useLocale } from "../../contexts/LocaleContext";
import type { KeepOptions, AuthStatus } from "../../ipc/apiDocs";
import type { ProviderInfo } from "../../ipc/provider";

interface Props {
  outputDir: string;
  onOutputDirChange: (v: string) => void;
  onPickFolder: () => void;
  merge: boolean;
  onMergeChange: (v: boolean) => void;
  keep: KeepOptions;
  onKeepChange: (v: KeepOptions) => void;
  auth: AuthStatus;
  domain: string;
  onLogin: () => void;
  onLogout: () => void;
  extracting: boolean;
  selectedCount: number;
  providers: ProviderInfo[];
  selectedProviderId: string;
  onProviderChange: (id: string) => void;
  translateToZh: boolean;
  onTranslateToZhChange: (v: boolean) => void;
  onExtractRaw: () => void;
  onExtractAi: () => void;
}

export function ExtractionSettings({
  outputDir, onOutputDirChange, onPickFolder,
  merge, onMergeChange,
  keep, onKeepChange,
  auth, domain, onLogin, onLogout,
  extracting, selectedCount,
  providers, selectedProviderId, onProviderChange,
  translateToZh, onTranslateToZhChange,
  onExtractRaw, onExtractAi,
}: Props) {
  const { t } = useLocale();
  const canExtract = selectedCount > 0 && !extracting;
  const hasProvider = selectedProviderId !== "";

  const toggleKeep = (key: keyof KeepOptions) => {
    onKeepChange({ ...keep, [key]: !keep[key] });
  };

  return (
    <div className="extraction-settings">
      {/* Output directory */}
      <div className="extraction-settings__section">
        <label className="extraction-settings__label">{t.api_docs_output_dir}</label>
        <div className="extraction-settings__dir-row">
          <input
            className="extraction-settings__input extraction-settings__input--flex"
            type="text"
            value={outputDir}
            onChange={(e) => onOutputDirChange(e.target.value)}
            placeholder={t.api_docs_output_dir_placeholder}
          />
          <button
            className="extraction-settings__btn extraction-settings__btn--icon"
            onClick={onPickFolder}
            title={t.api_docs_pick_folder}
            type="button"
          >
            📁
          </button>
        </div>
      </div>

      {/* Merge toggle */}
      <div className="extraction-settings__section">
        <label className="extraction-settings__checkbox-row">
          <input
            type="checkbox"
            checked={merge}
            onChange={(e) => onMergeChange(e.target.checked)}
          />
          <span>{t.api_docs_merge_label}</span>
        </label>
      </div>

      {/* Keep options */}
      <div className="extraction-settings__section">
        <div className="extraction-settings__label">{t.api_docs_keep_label}</div>
        {(
          [
            ["description", t.api_docs_keep_description],
            ["parameters", t.api_docs_keep_parameters],
            ["request_body", t.api_docs_keep_request_body],
            ["responses", t.api_docs_keep_responses],
            ["code_samples", t.api_docs_keep_code_samples],
          ] as [keyof KeepOptions, string][]
        ).map(([key, label]) => (
          <label key={key} className="extraction-settings__checkbox-row">
            <input
              type="checkbox"
              checked={keep[key]}
              onChange={() => toggleKeep(key)}
            />
            <span>{label}</span>
          </label>
        ))}
      </div>

      {/* Auth status */}
      {domain && (
        <div className="extraction-settings__section extraction-settings__auth">
          <div className="extraction-settings__label">{t.api_docs_auth_status_label}</div>
          <div className="extraction-settings__auth-row">
            <span
              className={`extraction-settings__auth-dot extraction-settings__auth-dot--${auth.logged_in ? "on" : "off"}`}
            />
            <span className="extraction-settings__auth-account">
              {auth.logged_in ? auth.account || domain : t.api_docs_not_logged_in}
            </span>
          </div>
          {auth.logged_in ? (
            <button className="extraction-settings__btn extraction-settings__btn--secondary" onClick={onLogout}>
              {t.api_docs_logout_btn}
            </button>
          ) : (
            <button className="extraction-settings__btn" onClick={onLogin}>
              {t.api_docs_login_btn}
            </button>
          )}
        </div>
      )}

      {/* AI provider selector + translate option */}
      <div className="extraction-settings__section">
        <label className="extraction-settings__label">{t.api_docs_ai_provider}</label>
        <select
          className="extraction-settings__select"
          value={selectedProviderId}
          onChange={(e) => onProviderChange(e.target.value)}
        >
          <option value="">{t.api_docs_no_provider}</option>
          {providers.map((p) => (
            <option key={p.id} value={p.id}>
              {p.display_name} ({p.model})
            </option>
          ))}
        </select>
        <label className="extraction-settings__checkbox-row" style={{ marginTop: "6px" }}>
          <input
            type="checkbox"
            checked={translateToZh}
            onChange={(e) => onTranslateToZhChange(e.target.checked)}
          />
          <span>{t.api_docs_translate_zh}</span>
        </label>
      </div>

      {/* Extract buttons */}
      <div className="extraction-settings__section extraction-settings__actions">
        {selectedCount === 0 && (
          <div className="extraction-settings__hint">{t.api_docs_no_pages}</div>
        )}
        <button
          className="extraction-settings__btn extraction-settings__btn--primary"
          disabled={!canExtract}
          onClick={onExtractRaw}
        >
          {extracting ? t.api_docs_extracting : t.api_docs_extract_raw}
        </button>
        <button
          className="extraction-settings__btn extraction-settings__btn--primary"
          disabled={!canExtract || !hasProvider}
          title={!hasProvider ? t.api_docs_no_provider : undefined}
          onClick={onExtractAi}
        >
          {t.api_docs_extract_ai}
        </button>
      </div>
    </div>
  );
}
