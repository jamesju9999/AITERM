import { confirm } from "@tauri-apps/plugin-dialog";
import { useLocale } from "../../contexts/LocaleContext";
import type { Notebook } from "../../ipc/knowledgeBase";
import type { SyncProgressState } from "../../hooks/useNotebooks";
import { SyncProgress } from "./SyncProgress";

interface Props {
  notebooks: Notebook[];
  activeId: string | null;
  syncingIds: Set<string>;
  syncProgressById: Record<string, SyncProgressState>;
  onSelect: (id: string) => void;
  onSync: (id: string) => void;
  onDelete: (id: string) => void;
  onAddClick: () => void;
}

export function NotebookSidebar({
  notebooks, activeId, syncingIds, syncProgressById, onSelect, onSync, onDelete, onAddClick,
}: Props) {
  const { t } = useLocale();

  const formatSyncedAt = (ts: number | null): string =>
    ts === null ? t.kb_never_synced : t.kb_last_synced(new Date(ts * 1000).toLocaleString());

  return (
    <div className="kb-sidebar">
      <div className="kb-sidebar__header">
        <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={onAddClick}>
          + {t.kb_create_notebook}
        </button>
      </div>

      <div className="kb-sidebar__list">
        {notebooks.length === 0 && (
          <div className="kb-sidebar__empty">{t.kb_no_notebooks}</div>
        )}
        {notebooks.map((nb) => {
          const isActive = nb.id === activeId;
          const isSyncing = syncingIds.has(nb.id);
          return (
            <div key={nb.id} className={`kb-sidebar__item ${isActive ? "kb-sidebar__item--active" : ""}`}>
              <button className="kb-sidebar__item-main" onClick={() => onSelect(nb.id)}>
                <div className="kb-sidebar__item-name">{nb.name}</div>
                <div className="kb-sidebar__item-path" title={nb.folder_path}>{nb.folder_path}</div>
                <div className="kb-sidebar__item-synced">
                  {isSyncing ? t.kb_syncing : formatSyncedAt(nb.last_synced_at)}
                </div>
              </button>

              {isSyncing && syncProgressById[nb.id] && <SyncProgress progress={syncProgressById[nb.id]} />}

              <div className="kb-sidebar__item-actions">
                <button
                  className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                  onClick={() => onSync(nb.id)}
                  disabled={isSyncing}
                >
                  {t.kb_sync_button}
                </button>
                <button
                  className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                  onClick={async () => {
                    // Not window.confirm: Tauri's webview has no JS dialog panel,
                    // so it returns without showing anything and the delete just
                    // happens. jsdom returns falsy, which is why tests never saw it.
                    if (await confirm(t.kb_delete_notebook_confirm(nb.name), {
                      kind: "warning",
                      okLabel: t.common_delete,
                      cancelLabel: t.common_cancel,
                    })) {
                      onDelete(nb.id);
                    }
                  }}
                >
                  ✕
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
