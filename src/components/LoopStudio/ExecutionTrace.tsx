import { useState, useEffect, useRef, useCallback } from "react";
import type { TraceEntry } from "../../hooks/useOrchestratorLoop";

interface ExecutionTraceProps {
  trace: TraceEntry[];
  isRunning: boolean;
  iteration: number;
}

export function ExecutionTrace({ trace, isRunning, iteration }: ExecutionTraceProps) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [autoScroll, setAutoScroll] = useState(true);
  const scrollRef = useRef<HTMLDivElement>(null);
  const isUserScrolling = useRef(false);

  const toggle = (id: string) => {
    setCollapsed(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  // Auto-scroll to bottom when trace updates
  useEffect(() => {
    if (!autoScroll || !scrollRef.current) return;
    scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [trace, autoScroll]);

  // Detect manual scroll — if user scrolls up, don't force them back down
  const handleScroll = useCallback(() => {
    if (!scrollRef.current || isUserScrolling.current) return;
    const el = scrollRef.current;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (!atBottom && autoScroll) {
      // User scrolled up manually — keep autoScroll state as-is (they toggled it themselves)
    }
  }, [autoScroll]);

  if (trace.length === 0) {
    return (
      <div className="ls-trace-wrap">
        <div className="ls-trace-toolbar">
          <label className="ls-autoscroll-toggle">
            <input type="checkbox" checked={autoScroll} onChange={e => setAutoScroll(e.target.checked)} />
            <span>自動捲動</span>
          </label>
        </div>
        <div className="ls-trace-empty">
          {isRunning ? "Loop 執行中..." : "執行記錄將在此顯示"}
        </div>
      </div>
    );
  }

  return (
    <div className="ls-trace-wrap">
      <div className="ls-trace-toolbar">
        <label className="ls-autoscroll-toggle">
          <input type="checkbox" checked={autoScroll} onChange={e => setAutoScroll(e.target.checked)} />
          <span>自動捲動</span>
        </label>
      </div>
      <div className="ls-trace" ref={scrollRef} onScroll={handleScroll}>
      {trace.map(entry => {
        const isCollapsed = collapsed.has(entry.id);

        if (entry.kind === "iteration_start") {
          return (
            <div key={entry.id} className="ls-trace-iteration">
              <span className="ls-trace-iter-badge">#{entry.iteration}</span>
              <span>Loop Iteration {entry.iteration}</span>
            </div>
          );
        }

        if (entry.kind === "verifier_result") {
          const vr = entry.verifierResult;
          const isCollapsedV = collapsed.has(entry.id);
          const hasDetails = vr && (vr.accomplished.length > 0 || vr.remaining.length > 0 || vr.suggestion);
          return (
            <div key={entry.id} className={`ls-trace-verifier ${entry.verifierDone ? "done" : "not-done"}`}>
              <div
                className="ls-verifier-header"
                onClick={() => hasDetails && toggle(entry.id)}
                style={{ cursor: hasDetails ? "pointer" : "default" }}
              >
                <span className="ls-verifier-badge">{entry.verifierDone ? "✓ 達成" : "✗ 未達成"}</span>
                <span className="ls-verifier-name">{entry.agentName}</span>
                <span className="ls-verifier-reason">{entry.text}</span>
                {hasDetails && (
                  <span className="ls-collapse-toggle">{isCollapsedV ? "▶" : "▼"}</span>
                )}
              </div>
              {hasDetails && !isCollapsedV && vr && (
                <div className="ls-verifier-details">
                  {vr.accomplished.length > 0 && (
                    <div className="ls-verifier-section">
                      <span className="ls-verifier-section-label accomplished">✓ 已完成</span>
                      <ul className="ls-verifier-list">
                        {vr.accomplished.map((a, i) => <li key={i}>{a}</li>)}
                      </ul>
                    </div>
                  )}
                  {vr.remaining.length > 0 && (
                    <div className="ls-verifier-section">
                      <span className="ls-verifier-section-label remaining">✗ 尚未完成</span>
                      <ul className="ls-verifier-list">
                        {vr.remaining.map((r, i) => <li key={i}>{r}</li>)}
                      </ul>
                    </div>
                  )}
                  {vr.suggestion && (
                    <div className="ls-verifier-section">
                      <span className="ls-verifier-section-label suggestion">→ 下一步建議</span>
                      <p className="ls-verifier-suggestion">{vr.suggestion}</p>
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        }

        if (entry.kind === "loop_done") {
          return (
            <div key={entry.id} className="ls-trace-done">
              {entry.text}
            </div>
          );
        }

        if (entry.kind === "loop_stopped" || entry.kind === "loop_error") {
          return (
            <div key={entry.id} className={`ls-trace-stopped ${entry.isError ? "error" : ""}`}>
              {entry.text}
            </div>
          );
        }

        if (entry.kind === "sub_agent_start") {
          return (
            <div key={entry.id} className="ls-trace-agent-start">
              <span className="ls-agent-badge">{entry.agentName}</span>
              <span className="ls-trace-task">{entry.text.replace(`${entry.agentName}: `, "")}</span>
            </div>
          );
        }

        if (entry.kind === "sub_agent_done") {
          const hasActions = entry.actions && entry.actions.length > 0;
          return (
            <div key={entry.id} className={`ls-trace-agent-done ${entry.isError ? "error" : ""}`}>
              <div
                className="ls-trace-collapsible-header"
                onClick={() => hasActions && toggle(entry.id)}
              >
                <span className="ls-agent-done-badge">{entry.isError ? "✗" : "✓"}</span>
                <span className="ls-agent-badge">{entry.agentName}</span>
                <span className="ls-trace-answer">{entry.text.slice(0, 120)}{entry.text.length > 120 ? "..." : ""}</span>
                {hasActions && (
                  <span className="ls-collapse-toggle">{isCollapsed ? "▶" : "▼"} {entry.actions!.length} 動作</span>
                )}
              </div>
              {hasActions && !isCollapsed && (
                <div className="ls-trace-actions">
                  {entry.actions!.map((action, i) => (
                    <div key={i} className={`ls-trace-action ${action.isError ? "error" : ""}`}>
                      <span className="ls-action-tool">{action.tool}</span>
                      <pre className="ls-action-input">{action.input}</pre>
                      <pre className="ls-action-output">{action.output.slice(0, 500)}{action.output.length > 500 ? "\n..." : ""}</pre>
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        }

        if (entry.kind === "orchestrator_action") {
          return (
            <div key={entry.id} className="ls-trace-orchestrator">
              <span className="ls-orch-badge">Orchestrator</span>
              <p className="ls-orch-text">{entry.text.slice(0, 300)}{entry.text.length > 300 ? "..." : ""}</p>
            </div>
          );
        }

        return null;
      })}

      {isRunning && (
        <div className="ls-trace-running">
          <span className="ls-spinner" /> 執行中（Loop #{iteration}）...
        </div>
      )}
    </div>
    </div>
  );
}
