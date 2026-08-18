import { useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import type { ActiveFeature } from "../../ipc/vcs";

interface Props {
  features: ActiveFeature[];
  loading: boolean;
  onRefresh: () => void;
  onStartFeature: () => void;
  onFinishFeature: (feature: ActiveFeature) => void;
}

export function TeamPanel({ features, loading, onRefresh, onStartFeature, onFinishFeature }: Props) {
  const { t } = useLocale();
  const [expandedNumber, setExpandedNumber] = useState<number | null>(null);

  return (
    <div className="vcs-team-panel">
      <div className="vcs-team-panel__header">
        <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={onStartFeature}>
          {t.vcs_start_feature}
        </button>
        <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onRefresh} disabled={loading}>
          {t.vcs_refresh}
        </button>
      </div>

      {features.length === 0 ? (
        <div className="vcs-team-panel__empty">{t.vcs_no_active_features}</div>
      ) : (
        <ul className="vcs-team-panel__list">
          {features.map((f) => {
            const expanded = expandedNumber === f.number;
            return (
              <li key={f.number} className="vcs-team-panel__item">
                <button
                  className="vcs-team-panel__item-main"
                  onClick={() => setExpandedNumber(expanded ? null : f.number)}
                >
                  <span className="vcs-team-panel__title">{f.title}</span>
                  <span className="vcs-team-panel__author">{f.author}</span>
                  <span className="vcs-team-panel__status">
                    {f.draft ? t.vcs_status_in_progress : t.vcs_status_in_review}
                  </span>
                </button>
                {expanded && (
                  <div className="vcs-team-panel__files">
                    <ul>
                      {f.files.map((file) => (
                        <li key={file}>{file}</li>
                      ))}
                    </ul>
                    {f.draft && (
                      <button
                        className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                        onClick={() => onFinishFeature(f)}
                      >
                        {t.vcs_finish_feature}
                      </button>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
