import { useEffect, useState } from "react";
import {
  getClaudeActivity,
  listenClaudeActivity,
  type ActivityNode,
} from "../../ipc/claudeActivity";

/**
 * 這個分頁 PTY 裡 Claude 的 subagent 清單。
 *
 * 先訂閱、再取快照：事件在兩者之間送出也不會漏。若事件已經先到，較舊的快照就不採用
 * （事件永遠是完整快照，比一次性的 invoke 結果新）。`alive` 防卸載後 setState
 * （Tauri listen 是非同步的，跟 useAiChat 的 mountedRef 同一個坑）。
 */
export function useClaudeActivity(sessionId: string): ActivityNode[] {
  const [nodes, setNodes] = useState<ActivityNode[]>([]);

  useEffect(() => {
    if (!sessionId) return;
    let alive = true;
    let gotEvent = false;
    let unlisten: (() => void) | undefined;
    setNodes([]);

    listenClaudeActivity(sessionId, (next) => {
      if (!alive) return;
      gotEvent = true;
      setNodes(next);
    })
      .then((u) => {
        if (alive) unlisten = u;
        else u();
      })
      .catch(() => {});

    getClaudeActivity(sessionId)
      .then((snap) => {
        if (alive && !gotEvent) setNodes(snap);
      })
      .catch(() => {});

    return () => {
      alive = false;
      unlisten?.();
    };
  }, [sessionId]);

  return nodes;
}
