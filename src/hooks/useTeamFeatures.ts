import { useCallback, useEffect, useRef, useState } from "react";
import { vcsListActiveFeatures, type ActiveFeature, type VcsRepoInfo } from "../ipc/vcs";

export interface UseTeamFeaturesResult {
  features: ActiveFeature[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

export function useTeamFeatures(repoInfo: VcsRepoInfo | null): UseTeamFeaturesResult {
  const [features, setFeatures] = useState<ActiveFeature[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const refresh = useCallback(async () => {
    if (!repoInfo) return;
    setLoading(true);
    setError(null);
    try {
      const list = await vcsListActiveFeatures(repoInfo);
      if (mountedRef.current) setFeatures(list);
    } catch (e) {
      if (mountedRef.current) setError(String(e));
    } finally {
      if (mountedRef.current) setLoading(false);
    }
  }, [repoInfo]);

  useEffect(() => { void refresh(); }, [refresh]);

  return { features, loading, error, refresh };
}
