import { useState, useEffect, useRef } from "react";
import { getSessionCwd } from "../ipc/fs";
import { vcsDetectRepo, type VcsRepoInfo } from "../ipc/vcs";

/**
 * Returns VcsRepoInfo for the current working directory.
 *
 * If `manualPath` is provided (non-empty), detects repo at that path and skips
 * PTY polling entirely. If `manualPath` is empty/null, falls back to polling
 * the PTY session CWD every 2 seconds.
 */
export function useVcsCwd(sessionId: string, manualPath?: string): VcsRepoInfo | null {
  const [repoInfo, setRepoInfo] = useState<VcsRepoInfo | null>(null);
  const lastCwdRef = useRef<string>("");

  // Manual path: detect once whenever manualPath changes, no polling
  useEffect(() => {
    if (!manualPath) return;
    lastCwdRef.current = manualPath;
    vcsDetectRepo(manualPath)
      .then(setRepoInfo)
      .catch(() => setRepoInfo(null));
  }, [manualPath]);

  // PTY polling: only active when no manual path is set
  useEffect(() => {
    if (manualPath) return;
    // Reset when switching back to auto mode
    lastCwdRef.current = "";
    const interval = setInterval(async () => {
      try {
        const cwd = await getSessionCwd(sessionId);
        if (cwd && cwd !== lastCwdRef.current) {
          lastCwdRef.current = cwd;
          try {
            const info = await vcsDetectRepo(cwd);
            setRepoInfo(info);
          } catch {
            setRepoInfo(null);
          }
        }
      } catch { /* ignore */ }
    }, 2000);
    return () => clearInterval(interval);
  }, [sessionId, manualPath]);

  return repoInfo;
}
