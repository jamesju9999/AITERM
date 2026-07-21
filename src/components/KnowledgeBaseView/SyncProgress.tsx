import { useLocale } from "../../contexts/LocaleContext";
import type { SyncProgressState } from "../../hooks/useNotebooks";

interface Props {
  progress: SyncProgressState;
}

export function SyncProgress({ progress }: Props) {
  const { t } = useLocale();
  const pct = progress.total > 0 ? Math.round((progress.processed / progress.total) * 100) : 0;

  return (
    <div className="kb-sync-progress">
      <div className="kb-sync-progress__label">
        {t.kb_sync_progress(progress.processed, progress.total)}
      </div>
      <div className="kb-sync-progress__bar">
        <div className="kb-sync-progress__fill" style={{ width: `${pct}%` }} />
      </div>
      {progress.currentFile && (
        <div className="kb-sync-progress__file" title={progress.currentFile}>
          {progress.currentFile}
        </div>
      )}
    </div>
  );
}
