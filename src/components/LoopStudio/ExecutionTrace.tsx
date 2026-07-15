import { useState, useEffect, useRef, useCallback } from "react";
import { formatTime, formatDuration } from "../../lib/timeFormat";
import type { TraceEntry } from "../../hooks/useOrchestratorLoop";
import { useLocale } from '../../contexts/LocaleContext';

function ClockBadge({ ts }: { ts: number }) {
  return <span className="ls-tb ls-tb-clock">{formatTime(ts)}</span>;
}

function DurBadge({ start, end, v }: { start: number; end: number; v?: boolean }) {
  return <span className={`ls-tb ${v ? "ls-tb-dur-v" : "ls-tb-dur"}`}>⏱ {formatDuration(start, end)}</span>;
}

function DoneBadge({ ts }: { ts: number }) {
  return <span className="ls-tb ls-tb-done">✓ {formatTime(ts)}</span>;
}

interface ExecutionTraceProps {
  trace: TraceEntry[];
  isRunning: boolean;
  iteration: number;
  timingMode: "full" | "compact";
}

export function ExecutionTrace({ trace, isRunning, iteration, timingMode }: ExecutionTraceProps) {
  const { t } = useLocale();
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [autoScroll, setAutoScroll] = useState(true);
  const scrollRef = useRef<HTMLDivElement>(null);
  const isProgrammaticScroll = useRef(false);

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
    isProgrammaticScroll.current = true;
    scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    requestAnimationFrame(() => { isProgrammaticScroll.current = false; });
  }, [trace, autoScroll]);

  // Detect manual scroll — if user scrolls up, don't force them back down
  const handleScroll = useCallback(() => {
    if (!scrollRef.current) return;
    const el = scrollRef.current;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (!atBottom && autoScroll && !isProgrammaticScroll.current) {
      setAutoScroll(false);
    }
  }, [autoScroll]);

  if (trace.length === 0) {
    return (
      <div className="ls-trace-wrap">
        <div className="ls-trace-toolbar">
          <label className="ls-autoscroll-toggle">
            <input type="checkbox" checked={autoScroll} onChange={e => setAutoScroll(e.target.checked)} />
            <span>{t.ls_auto_scroll}</span>
          </label>
        </div>
        <div className="ls-trace-empty">
          {isRunning ? t.ls_trace_running : t.ls_trace_idle}
        </div>
      </div>
    );
  }

  return (
    <div className="ls-trace-wrap">
      <div className="ls-trace-toolbar">
        <label className="ls-autoscroll-toggle">
          <input type="checkbox" checked={autoScroll} onChange={e => setAutoScroll(e.target.checked)} />
          <span>{t.ls_auto_scroll}</span>
        </label>
      </div>
      <div className="ls-trace" ref={scrollRef} onScroll={handleScroll}>
      {(() => {
        const loopStartTs = trace.find(e => e.kind === "iteration_start")?.timestamp;
        return trace.map(entry => {
        if (entry.kind === "iteration_start") {
          return (
            <div key={entry.id} className="ls-trace-iteration ls-entry-timed">
              <div className="ls-entry-body">
                <span className="ls-trace-iter-badge">#{entry.iteration}</span>
                <span>Loop Iteration {entry.iteration}</span>
              </div>
              <div className="ls-time-meta">
                <ClockBadge ts={entry.timestamp} />
              </div>
            </div>
          );
        }

        if (entry.kind === "verifier_result") {
          const vr = entry.verifierResult;
          const isCollapsedV = collapsed.has(entry.id);
          const hasDetails = vr && (vr.accomplished.length > 0 || vr.remaining.length > 0 || vr.suggestion);
          return (
            <div key={entry.id} className={`ls-trace-verifier ${entry.verifierDone ? "done" : "not-done"} ls-entry-timed`}>
              <div className="ls-entry-body">
                <div
                  className="ls-verifier-header"
                  onClick={() => hasDetails && toggle(entry.id)}
                  style={{ cursor: hasDetails ? "pointer" : "default" }}
                >
                  <span className="ls-verifier-badge">{entry.verifierDone ? t.ls_verifier_done_badge : t.ls_verifier_not_done_badge}</span>
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
                        <span className="ls-verifier-section-label accomplished">{t.ls_verifier_accomplished}</span>
                        <ul className="ls-verifier-list">
                          {vr.accomplished.map((a, i) => <li key={i}>{a}</li>)}
                        </ul>
                      </div>
                    )}
                    {vr.remaining.length > 0 && (
                      <div className="ls-verifier-section">
                        <span className="ls-verifier-section-label remaining">{t.ls_verifier_remaining}</span>
                        <ul className="ls-verifier-list">
                          {vr.remaining.map((r, i) => <li key={i}>{r}</li>)}
                        </ul>
                      </div>
                    )}
                    {vr.suggestion && (
                      <div className="ls-verifier-section">
                        <span className="ls-verifier-section-label suggestion">{t.ls_verifier_suggestion}</span>
                        <p className="ls-verifier-suggestion">{vr.suggestion}</p>
                      </div>
                    )}
                  </div>
                )}
              </div>
              <div className="ls-time-meta">
                {entry.startTimestamp != null && (
                  <DurBadge start={entry.startTimestamp} end={entry.timestamp} v />
                )}
                <DoneBadge ts={entry.timestamp} />
              </div>
            </div>
          );
        }

        if (entry.kind === "loop_done") {
          return (
            <div key={entry.id} className="ls-trace-done ls-entry-timed">
              <div className="ls-entry-body">{entry.text}</div>
              <div className="ls-time-meta">
                {loopStartTs != null && (
                  <span className="ls-tb ls-tb-total">{t.ls_total_duration(formatDuration(loopStartTs, entry.timestamp))}</span>
                )}
                <DoneBadge ts={entry.timestamp} />
              </div>
            </div>
          );
        }

        if (entry.kind === "loop_stopped" || entry.kind === "loop_error") {
          return (
            <div key={entry.id} className={`ls-trace-stopped ${entry.isError ? "error" : ""} ls-entry-timed`}>
              <div className="ls-entry-body">{entry.text}</div>
              <div className="ls-time-meta">
                <ClockBadge ts={entry.timestamp} />
              </div>
            </div>
          );
        }

        if (entry.kind === "sub_agent_start") {
          return (
            <div key={entry.id} className="ls-trace-agent-start ls-entry-timed">
              <div className="ls-entry-body">
                <span className="ls-agent-badge">{entry.agentName}</span>
                <span className="ls-trace-task">{t.ls_agent_received}</span>
              </div>
              <div className="ls-time-meta">
                <ClockBadge ts={entry.timestamp} />
              </div>
            </div>
          );
        }

        if (entry.kind === "sub_agent_action") {
          const action = entry.actions?.[0];
          if (!action) return null;
          return (
            <div key={entry.id} className={`ls-trace-live-action ${action.isError ? "error" : ""} ls-entry-timed`}>
              <div className="ls-entry-body">
                <span className="ls-agent-badge">{entry.agentName}</span>
                <div className="ls-trace-action">
                  <span className="ls-action-tool">{action.tool}</span>
                  <pre className="ls-action-input">{action.input}</pre>
                  <pre className="ls-action-output">{action.output.slice(0, 500)}{action.output.length > 500 ? "\n..." : ""}</pre>
                </div>
              </div>
              {timingMode === "full" && (
                <div className="ls-time-meta">
                  <ClockBadge ts={entry.timestamp} />
                </div>
              )}
            </div>
          );
        }

        if (entry.kind === "sub_agent_done") {
          // Actions were already shown live via sub_agent_action entries above,
          // so this only needs to report the agent's final answer.
          return (
            <div key={entry.id} className={`ls-trace-agent-done ${entry.isError ? "error" : ""} ls-entry-timed`}>
              <div className="ls-entry-body">
                <div className="ls-trace-collapsible-header" style={{ cursor: "default" }}>
                  <span className="ls-agent-done-badge">{entry.isError ? "✗" : "✓"}</span>
                  <span className="ls-agent-badge">{entry.agentName}</span>
                  <span className="ls-trace-answer">{entry.text.slice(0, 120)}{entry.text.length > 120 ? "..." : ""}</span>
                </div>
              </div>
              <div className="ls-time-meta">
                {entry.startTimestamp != null && (
                  <DurBadge start={entry.startTimestamp} end={entry.timestamp} />
                )}
                <DoneBadge ts={entry.timestamp} />
              </div>
            </div>
          );
        }

        if (entry.kind === "orchestrator_action") {
          return (
            <div key={entry.id} className="ls-trace-orchestrator ls-entry-timed">
              <div className="ls-entry-body">
                <span className="ls-orch-badge">Orchestrator</span>
                <p className="ls-orch-text">{entry.text.slice(0, 300)}{entry.text.length > 300 ? "..." : ""}</p>
              </div>
              {timingMode === "full" && (
                <div className="ls-time-meta">
                  <ClockBadge ts={entry.timestamp} />
                </div>
              )}
            </div>
          );
        }

        return null;
      });
      })()}

      {isRunning && (
        <div className="ls-trace-running">
          <span className="ls-spinner" /> {t.ls_loop_running_status(iteration)}
        </div>
      )}
    </div>
    </div>
  );
}
