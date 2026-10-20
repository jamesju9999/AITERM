import type { ActivityNode, ActivityStatus } from "../../ipc/claudeActivity";
import { useLocale } from "../../contexts/LocaleContext";
import { formatDuration, formatTokens } from "./format";

/** 選中節點的詳情；沒選時顯示提示。圖與時間軸共用。 */
export function DetailPane({ node, now }: { node: ActivityNode | null; now: number }) {
  const { t } = useLocale();
  if (!node) {
    return (
      <div className="aiterm-cact__detail">
        <div className="aiterm-cact__hint">{t.claude_activity_pick}</div>
      </div>
    );
  }
  const statusLabel: Record<ActivityStatus, string> = {
    running: t.claude_activity_status_running,
    done: t.claude_activity_status_done,
    failed: t.claude_activity_status_failed,
    interrupted: t.claude_activity_status_interrupted,
  };
  return (
    <div className="aiterm-cact__detail">
      <div className="aiterm-cact__detail-title">{node.description ?? t.claude_activity_untitled}</div>
      <div className="aiterm-cact__meta">
        <span className="aiterm-cact__badge">{node.agent_type}</span>
        <span>{statusLabel[node.status]}</span>
        <span>{formatDuration((node.ended_ms ?? now) - node.started_ms)}</span>
        {node.tokens != null && <span>{t.claude_activity_tokens(formatTokens(node.tokens))}</span>}
        {node.tool_count != null && <span>{t.claude_activity_tools(node.tool_count)}</span>}
      </div>
      {node.status === "running" && node.current_tool && <div className="aiterm-cact__tool">{node.current_tool}</div>}
      {node.summary && <p className="aiterm-cact__summary">{node.summary}</p>}
    </div>
  );
}
