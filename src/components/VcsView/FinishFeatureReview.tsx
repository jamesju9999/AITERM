import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { vcsFinishFeature, vcsGetFeatureDiff, vcsMergeFeature, type ActiveFeature, type VcsRepoInfo } from "../../ipc/vcs";

interface Props {
  repoInfo: VcsRepoInfo;
  feature: ActiveFeature;
  onSubmittedForReview: () => void;
  onMerged: () => void;
  onClose: () => void;
}

export function FinishFeatureReview({ repoInfo, feature, onSubmittedForReview, onMerged, onClose }: Props) {
  const { t } = useLocale();
  const [diff, setDiff] = useState<string | null>(null);
  const [isDraft, setIsDraft] = useState(feature.draft);
  const [deleteBranch, setDeleteBranch] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setDiff(null);
    setError(null);
    vcsGetFeatureDiff(repoInfo, feature.base_ref, feature.head_ref)
      .then((d) => {
        if (!cancelled) setDiff(d);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [repoInfo.root, repoInfo.connection_id, feature.base_ref, feature.head_ref]);

  const handleSubmitForReview = async () => {
    setBusy(true);
    setError(null);
    try {
      await vcsFinishFeature(repoInfo, feature.number);
      setIsDraft(false);
      onSubmittedForReview();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleMerge = async () => {
    setBusy(true);
    setError(null);
    try {
      await vcsMergeFeature(repoInfo, feature.number, deleteBranch ? feature.head_ref : null);
      onMerged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="vcs-finish-review-overlay">
      <div className="vcs-finish-review-dialog">
        <h3>{feature.title}</h3>

        {diff === null && !error ? (
          <div>{t.vcs_loading_diff}</div>
        ) : (
          <pre className="vcs-diff-view">{diff}</pre>
        )}

        {error && <div className="vcs-finish-review-dialog__error">{error}</div>}

        {!isDraft && (
          <label className="vcs-delete-branch-checkbox">
            <input
              type="checkbox"
              checked={deleteBranch}
              onChange={(e) => setDeleteBranch(e.target.checked)}
              disabled={busy}
            />
            {t.vcs_delete_branch_after_merge}
          </label>
        )}

        <div className="vcs-finish-review-dialog__actions">
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={onClose} disabled={busy}>
            {t.common_cancel}
          </button>
          {isDraft ? (
            <button
              className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
              onClick={handleSubmitForReview}
              disabled={busy || diff === null}
            >
              {t.vcs_submit_for_review}
            </button>
          ) : (
            <button
              className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
              onClick={handleMerge}
              disabled={busy || diff === null}
            >
              {t.vcs_merge_button}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
