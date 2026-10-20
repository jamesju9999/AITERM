import { useEffect, useState } from "react";
import {
  EMPTY_SNAPSHOT,
  getClaudeActivity,
  listenClaudeActivity,
  type ActivitySnapshot,
} from "../../ipc/claudeActivity";

/**
 * 這個分頁 PTY 裡 Claude 的 subagent 清單。
 *
 * 先訂閱、再取快照：事件在兩者之間送出也不會漏。若事件已經先到，較舊的快照就不採用
 * （事件永遠是完整快照，比一次性的 invoke 結果新）。`alive` 防卸載後 setState
 * （Tauri listen 是非同步的，跟 useAiChat 的 mountedRef 同一個坑）。
 */
export function useClaudeActivity(sessionId: string): ActivitySnapshot {
  // 連同 sessionId 一起存：換 session 時舊清單自然對不上而被忽略，不必在 effect 裡同步清空。
  const [state, setState] = useState<{ sessionId: string; snapshot: ActivitySnapshot }>({ sessionId: "", snapshot: EMPTY_SNAPSHOT });

  useEffect(() => {
    if (!sessionId) return;
    let alive = true;
    let gotEvent = false;
    let unlisten: (() => void) | undefined;

    listenClaudeActivity(sessionId, (next) => {
      if (!alive) return;
      gotEvent = true;
      setState({ sessionId, snapshot: next });
    })
      .then((u) => {
        if (alive) unlisten = u;
        else u();
      })
      .catch(() => {});

    getClaudeActivity(sessionId)
      .then((snap) => {
        if (alive && !gotEvent) setState({ sessionId, snapshot: snap });
      })
      .catch(() => {});

    return () => {
      alive = false;
      unlisten?.();
    };
  }, [sessionId]);

  return state.sessionId === sessionId ? state.snapshot : EMPTY_SNAPSHOT;
}
