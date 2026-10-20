import type { ActivityNode, ActivityStatus } from "../../ipc/claudeActivity";
import { useLocale } from "../../contexts/LocaleContext";
import { formatDuration, formatTokens } from "./format";
import { isToolId } from "./toolNodes";

export interface MainActivity {
  count: number;
  recent: string[];
}

/** 選中節點的詳情；沒選時顯示提示。選到根時顯示主 Claude 的活動。圖與時間軸共用。 */
export function DetailPane({ node, now, share = null, main = null }: { node: ActivityNode | null; now: number; share?: number | null; main?: MainActivity | null }) {
  const { t } = useLocale();
  if (main) {
    return (
      <div className="aiterm-cact__detail">
        <div className="aiterm-cact__detail-title">{t.claude_activity_main_tools(main.count)}</div>
        {main.recent.length > 0 && (
          <>
            <div className="aiterm-cact__hint">{t.claude_activity_main_recent}</div>
            <ul className="aiterm-cact__recent">
              {main.recent.map((r, i) => <li key={i}>{r}</li>)}
            </ul>
          </>
        )}
      </div>
    );
  }
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
  if (isToolId(node.id)) {
    const category = t[`claude_activity_cat_${node.agent_type}` as keyof typeof t];
    return (
      <div className="aiterm-cact__detail">
        <div className="aiterm-cact__detail-title">{node.description}</div>
        <div className="aiterm-cact__meta">
          <span className="aiterm-cact__badge">{typeof category === "string" ? category : t.claude_activity_cat_other}</span>
          <span>{statusLabel[node.status]}</span>
          <span>{formatDuration((node.ended_ms ?? now) - node.started_ms)}</span>
        </div>
      </div>
    );
  }
  return (
    <div className="aiterm-cact__detail">
      <div className="aiterm-cact__detail-title">{node.description ?? t.claude_activity_untitled}</div>
      <div className="aiterm-cact__meta">
        <span className="aiterm-cact__badge">{node.agent_type}</span>
        <span>{statusLabel[node.status]}</span>
        <span>{formatDuration((node.ended_ms ?? now) - node.started_ms)}</span>
        {node.tokens != null && <span>{t.claude_activity_tokens(formatTokens(node.tokens))}</span>}
        {share != null && <span>{t.claude_activity_share(share)}</span>}
        {node.tool_count != null && <span>{t.claude_activity_tools(node.tool_count)}</span>}
      </div>
      {node.status === "running" && node.current_tool && <div className="aiterm-cact__tool">{node.current_tool}</div>}
      {node.summary && <p className="aiterm-cact__summary">{node.summary}</p>}
    </div>
  );
}
