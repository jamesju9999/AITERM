import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { vcsCheckOverlap, vcsGetDefaultBranch, vcsStartFeature, type ActiveFeature, type VcsRepoInfo } from "../../ipc/vcs";

interface Props {
  repoInfo: VcsRepoInfo;
  onStarted: () => void;
  onClose: () => void;
}

function parseFileList(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

export function StartFeatureDialog({ repoInfo, onStarted, onClose }: Props) {
  const { t } = useLocale();
  const [name, setName] = useState("");
  const [filesText, setFilesText] = useState("");
  const [overlaps, setOverlaps] = useState<ActiveFeature[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [baseBranch, setBaseBranch] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    vcsGetDefaultBranch(repoInfo)
      .then((branch) => {
        if (!cancelled) setBaseBranch(branch);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [repoInfo.root, repoInfo.connection_id]);

  const doStart = async () => {
    if (baseBranch === null) return;
    setBusy(true);
    setError(null);
    try {
      await vcsStartFeature(repoInfo, name, baseBranch, parseFileList(filesText));
      onStarted();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleSubmit = async () => {
    if (baseBranch === null) return;
    const declaredFiles = parseFileList(filesText);
    if (declaredFiles.length === 0) {
      await doStart();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const hits = await vcsCheckOverlap(repoInfo, declaredFiles);
      if (hits.length > 0) {
        setOverlaps(hits);
      } else {
        await doStart();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="vcs-start-feature-overlay" onClick={onClose}>
      <div className="vcs-start-feature-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="vcs-start-feature-dialog__title">{t.vcs_start_feature}</h3>

        <label className="vcs-start-feature-dialog__field">
          <span>{t.vcs_feature_name_label}</span>
          <input value={name} onChange={(e) => setName(e.target.value)} disabled={busy} />
        </label>

        <label className="vcs-start-feature-dialog__field">
          <span>{t.vcs_feature_files_label}</span>
          <textarea
            value={filesText}
            onChange={(e) => {
              setFilesText(e.target.value);
              setOverlaps(null);
            }}
            disabled={busy}
            rows={4}
            placeholder={t.vcs_feature_files_placeholder}
          />
        </label>

        {overlaps && overlaps.length > 0 && (
          <div className="vcs-overlap-warning">
            <p>{t.vcs_overlap_warning}</p>
            <ul>
              {overlaps.map((f) => (
                <li key={f.number}>{t.vcs_overlap_entry(f.author, f.title)}</li>
              ))}
            </ul>
            <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={doStart} disabled={busy || baseBranch === null}>
              {t.vcs_start_anyway}
            </button>
          </div>
        )}

        {baseBranch === null && !error && <div className="vcs-start-feature-dialog__hint">{t.vcs_detecting_base_branch}</div>}

        {error && <div className="vcs-start-feature-dialog__error">{error}</div>}

        <div className="vcs-start-feature-dialog__actions">
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onClose} disabled={busy}>
            {t.common_cancel}
          </button>
          {!overlaps && (
            <button
              className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
              onClick={handleSubmit}
              disabled={busy || !name.trim() || baseBranch === null}
            >
              {t.vcs_start_feature}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
