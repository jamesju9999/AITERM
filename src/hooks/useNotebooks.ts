import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  kbCreateNotebook, kbListNotebooks, kbDeleteNotebook, kbSyncNotebook,
  KB_SYNC_EVENT, type Notebook, type KbSyncEvent, type SyncSummary,
} from "../ipc/knowledgeBase";

export interface SyncProgressState {
  processed: number;
  total: number;
  currentFile: string;
}

export interface UseNotebooksResult {
  notebooks: Notebook[];
  loading: boolean;
  error: string | null;
  syncingId: string | null;
  syncProgress: SyncProgressState | null;
  refresh: () => Promise<void>;
  create: (name: string, folderPath: string, embedProviderId?: string, embedModel?: string) => Promise<Notebook>;
  remove: (id: string) => Promise<void>;
  sync: (id: string) => Promise<SyncSummary>;
}

export function useNotebooks(): UseNotebooksResult {
  const [notebooks, setNotebooks] = useState<Notebook[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [syncingId, setSyncingId] = useState<string | null>(null);
  const [syncProgress, setSyncProgress] = useState<SyncProgressState | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const list = await kbListNotebooks();
      if (mountedRef.current) setNotebooks(list);
    } catch (e) {
      if (mountedRef.current) setError(String(e));
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => { void refresh(); }, [refresh]);

  const create = useCallback(async (
    name: string, folderPath: string, embedProviderId?: string, embedModel?: string,
  ) => {
    const nb = await kbCreateNotebook(name, folderPath, embedProviderId, embedModel);
    await refresh();
    return nb;
  }, [refresh]);

  const remove = useCallback(async (id: string) => {
    await kbDeleteNotebook(id);
    await refresh();
  }, [refresh]);

  const sync = useCallback(async (id: string) => {
    setSyncingId(id);
    setSyncProgress({ processed: 0, total: 0, currentFile: "" });

    const unlisten = await listen<KbSyncEvent>(KB_SYNC_EVENT, (event) => {
      if (!mountedRef.current) return;
      const p = event.payload;
      if (p.notebook_id !== id) return;
      if (p.kind === "progress") {
        setSyncProgress({ processed: p.processed, total: p.total, currentFile: p.current_file });
      }
    });

    try {
      const summary = await kbSyncNotebook(id);
      await refresh();
      return summary;
    } finally {
      unlisten();
      if (mountedRef.current) {
        setSyncingId(null);
        setSyncProgress(null);
      }
    }
  }, [refresh]);

  return { notebooks, loading, error, syncingId, syncProgress, refresh, create, remove, sync };
}
