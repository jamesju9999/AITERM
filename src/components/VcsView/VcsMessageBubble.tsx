import { useNavigate } from "react-router-dom";
import type { VcsChatMessage } from "../../hooks/useVcsChat";
import type { VcsLoopMessage } from "../../hooks/useVcsAgentLoop";
import type { VcsWriteMode, CommitEntry, PrEntry, WorkflowRun, IssueEntry, BranchEntry, BlameEntry } from "../../ipc/vcs";
import { useLocale } from "../../contexts/LocaleContext";

interface VcsMessageBubbleProps {
  message: VcsChatMessage;
  writeMode: VcsWriteMode;
  onAction: (action: string, data: unknown) => void;
  onConfirmWrite: (intent: unknown) => void;
  onCancelWrite: () => void;
}

const isReadOnly = (writeMode: VcsWriteMode) => writeMode === "read_only";

function CommitCard({ commit, writeMode, onAction }: { commit: CommitEntry; writeMode: VcsWriteMode; onAction: (action: string, data: unknown) => void }) {
  const { t } = useLocale();
  return (
    <div style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6, padding: "10px 12px", marginBottom: 6 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "baseline", marginBottom: 4 }}>
        <span style={{ color: "#a78bfa", fontFamily: "monospace", fontSize: 12 }}>{commit.revision.slice(0, 7)}</span>
        <span style={{ color: "#888", fontSize: 11 }}>{commit.author}</span>
        <span style={{ color: "#555", fontSize: 11 }}>{commit.date}</span>
      </div>
      <div style={{ color: "#e6e6e6", fontSize: 13, marginBottom: 4 }}>{commit.message}</div>
      <div style={{ color: "#666", fontSize: 11, marginBottom: 8 }}>{t.vcs_files_changed(commit.files_changed.length)}</div>
      <div style={{ display: "flex", gap: 6 }}>
        <button
          onClick={() => onAction("view_diff", commit)}
          style={smallBtnStyle}
        >
          {t.vcs_view_diff}
        </button>
        <button
          onClick={() => onAction("revert", commit)}
          disabled={isReadOnly(writeMode)}
          title={isReadOnly(writeMode) ? t.vcs_read_only_tooltip : undefined}
          style={{ ...smallBtnStyle, opacity: isReadOnly(writeMode) ? 0.4 : 1 }}
        >
          {t.vcs_revert}
        </button>
      </div>
    </div>
  );
}

function PrCard({ pr, writeMode, onAction }: { pr: PrEntry; writeMode: VcsWriteMode; onAction: (action: string, data: unknown) => void }) {
  const { t } = useLocale();
  const stateColor = pr.state === "open" ? "#34d399" : pr.state === "merged" ? "#a78bfa" : "#888";
  return (
    <div style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6, padding: "10px 12px", marginBottom: 6 }}>
      <div style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 4 }}>
        <span style={{ color: "#888", fontSize: 11 }}>#{pr.number}</span>
        <span style={{ background: stateColor + "22", color: stateColor, fontSize: 10, padding: "1px 6px", borderRadius: 10, border: `1px solid ${stateColor}44` }}>{pr.state}</span>
      </div>
      <div style={{ color: "#e6e6e6", fontSize: 13, marginBottom: 2 }}>{pr.title}</div>
      <div style={{ color: "#888", fontSize: 11, marginBottom: 8 }}>{pr.author} · {pr.updated_at}</div>
      <div style={{ display: "flex", gap: 6 }}>
        <button onClick={() => onAction("view_pr_comments", pr)} style={smallBtnStyle}>
          {t.vcs_btn_comments}
        </button>
        <button
          onClick={() => onAction("merge_pr", pr)}
          disabled={isReadOnly(writeMode)}
          title={isReadOnly(writeMode) ? t.vcs_read_only_tooltip : undefined}
          style={{ ...smallBtnStyle, opacity: isReadOnly(writeMode) ? 0.4 : 1 }}
        >
          {t.vcs_merge_pr}
        </button>
      </div>
    </div>
  );
}

function statusBadgeColor(status: string, conclusion?: string | null): string {
  if (conclusion === "success") return "#34d399";
  if (conclusion === "failure") return "#f87171";
  if (status === "in_progress") return "#fbbf24";
  return "#888";
}

export function VcsMessageBubble({ message, writeMode, onAction, onConfirmWrite, onCancelWrite }: VcsMessageBubbleProps) {
  const { t } = useLocale();
  const navigate = useNavigate();

  if (message.role === "user") {
    return (
      <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 8 }}>
        <div style={{ background: "#1e3a2e", border: "1px solid #34d39944", borderRadius: "8px 8px 2px 8px", padding: "8px 12px", maxWidth: "80%", color: "#e6e6e6", fontSize: 13 }}>
          {message.text}
        </div>
      </div>
    );
  }

  const result = message.result;

  if (!result) {
    return (
      <div style={{ marginBottom: 8 }}>
        <div style={{ color: "#888", fontSize: 13, padding: "8px 12px" }}>{message.text}</div>
      </div>
    );
  }

  const wrapper = (children: React.ReactNode) => (
    <div style={{ marginBottom: 12 }}>{children}</div>
  );

  if (result.type === "log") {
    return wrapper(
      <>
        {result.commits.map((c) => (
          <CommitCard key={c.revision} commit={c} writeMode={writeMode} onAction={onAction} />
        ))}
        {result.truncated && <div style={{ color: "#555", fontSize: 11, textAlign: "center", padding: "4px 0" }}>{t.vcs_log_truncated}</div>}
      </>
    );
  }

  if (result.type === "diff") {
    return wrapper(
      <pre style={{ background: "#0f0f0f", border: "1px solid #2a2a2a", borderRadius: 6, padding: 12, fontSize: 12, color: "#e6e6e6", overflowX: "auto", whiteSpace: "pre-wrap", wordBreak: "break-all" }}>
        {result.content}
      </pre>
    );
  }

  if (result.type === "blame") {
    return wrapper(
      <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 11, fontFamily: "monospace" }}>
        <thead>
          <tr style={{ color: "#666", borderBottom: "1px solid #2a2a2a" }}>
            <th style={{ padding: "4px 8px", textAlign: "right" }}>{t.vcs_blame_col_line}</th>
            <th style={{ padding: "4px 8px", textAlign: "left" }}>{t.vcs_blame_col_rev}</th>
            <th style={{ padding: "4px 8px", textAlign: "left" }}>{t.vcs_blame_col_author}</th>
            <th style={{ padding: "4px 8px", textAlign: "left" }}>{t.vcs_blame_col_date}</th>
            <th style={{ padding: "4px 8px", textAlign: "left" }}>{t.vcs_blame_col_content}</th>
          </tr>
        </thead>
        <tbody>
          {(result.lines as BlameEntry[]).map((line) => (
            <tr key={line.line_number} style={{ borderBottom: "1px solid #1a1a1a" }}>
              <td style={{ padding: "2px 8px", color: "#555", textAlign: "right" }}>{line.line_number}</td>
              <td style={{ padding: "2px 8px", color: "#a78bfa" }}>{line.revision.slice(0, 7)}</td>
              <td style={{ padding: "2px 8px", color: "#888" }}>{line.author}</td>
              <td style={{ padding: "2px 8px", color: "#666" }}>{line.date}</td>
              <td style={{ padding: "2px 8px", color: "#e6e6e6" }}>{line.content}</td>
            </tr>
          ))}
        </tbody>
      </table>
    );
  }

  if (result.type === "branches") {
    return wrapper(
      <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
        {(result.branches as BranchEntry[]).map((b) => (
          <div key={b.name} style={{ display: "flex", gap: 8, alignItems: "center", padding: "4px 8px", background: b.is_current ? "#1e3a2e" : "transparent", borderRadius: 4 }}>
            {b.is_current && <span style={{ color: "#34d399", fontSize: 10 }}>●</span>}
            <span style={{ color: b.is_current ? "#34d399" : "#e6e6e6", fontSize: 13 }}>{b.name}</span>
            {b.is_remote && <span style={{ color: "#555", fontSize: 10 }}>remote</span>}
          </div>
        ))}
      </div>
    );
  }

  if (result.type === "pr_list") {
    return wrapper(
      <>
        {(result.prs as PrEntry[]).map((pr) => (
          <PrCard key={pr.number} pr={pr} writeMode={writeMode} onAction={onAction} />
        ))}
        {result.prs.length === 0 && <div style={{ color: "#555", fontSize: 13 }}>{t.vcs_empty_prs}</div>}
      </>
    );
  }

  if (result.type === "issue_list") {
    return wrapper(
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {(result.issues as IssueEntry[]).map((issue) => (
          <div key={issue.number} style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6, padding: "8px 12px" }}>
            <div style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 2 }}>
              <span style={{ color: "#888", fontSize: 11 }}>#{issue.number}</span>
              <span style={{ color: issue.state === "open" ? "#34d399" : "#888", fontSize: 10, background: "#2a2a2a", padding: "1px 6px", borderRadius: 10 }}>{issue.state}</span>
            </div>
            <div style={{ color: "#e6e6e6", fontSize: 13 }}>{issue.title}</div>
            <div style={{ color: "#666", fontSize: 11, marginTop: 2 }}>{issue.author} · {issue.created_at}</div>
          </div>
        ))}
        {result.issues.length === 0 && <div style={{ color: "#555", fontSize: 13 }}>{t.vcs_empty_issues}</div>}
      </div>
    );
  }

  if (result.type === "actions_list") {
    return wrapper(
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {(result.runs as WorkflowRun[]).map((run) => {
          const color = statusBadgeColor(run.status, run.conclusion);
          return (
            <div key={run.id} style={{ background: "#1a1a1a", border: "1px solid #2a2a2a", borderRadius: 6, padding: "8px 12px" }}>
              <div style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 2 }}>
                <span style={{ color, fontSize: 10 }}>●</span>
                <span style={{ color: "#e6e6e6", fontSize: 13 }}>{run.name}</span>
              </div>
              <div style={{ color: "#666", fontSize: 11 }}>{run.status}{run.conclusion ? ` · ${run.conclusion}` : ""} · {run.created_at}</div>
            </div>
          );
        })}
        {result.runs.length === 0 && <div style={{ color: "#555", fontSize: 13 }}>{t.vcs_empty_workflows}</div>}
      </div>
    );
  }

  if (result.type === "write_confirm") {
    return wrapper(
      <div style={{ background: "#1a1400", border: "1px solid #f9a82544", borderRadius: 6, padding: 12 }}>
        <div style={{ color: "#f9a825", fontSize: 13, fontWeight: 500, marginBottom: 6 }}>{t.vcs_confirm_operation(result.operation)}</div>
        <pre style={{ background: "#0f0f0f", borderRadius: 4, padding: 8, fontSize: 12, color: "#e6e6e6", overflowX: "auto", whiteSpace: "pre-wrap", marginBottom: 10 }}>{result.preview}</pre>
        <div style={{ display: "flex", gap: 8 }}>
          <button onClick={() => onConfirmWrite(result.intent)} style={{ ...smallBtnStyle, borderColor: "#34d399", color: "#34d399" }}>{t.vcs_confirm_execute}</button>
          <button onClick={onCancelWrite} style={smallBtnStyle}>{t.vcs_confirm_cancel}</button>
        </div>
      </div>
    );
  }

  if (result.type === "write_success") {
    return wrapper(
      <div style={{ background: "#1e3a2e", border: "1px solid #34d39944", borderRadius: 6, padding: "8px 12px", color: "#34d399", fontSize: 13 }}>
        ✓ {result.operation}：{result.detail}
      </div>
    );
  }

  if (result.type === "error") {
    return wrapper(
      <div style={{ background: "#2a0f0f", border: "1px solid #f8717144", borderRadius: 6, padding: "8px 12px", color: "#f87171", fontSize: 13 }}>
        ✗ {result.message}
      </div>
    );
  }

  if (result.type === "no_token") {
    return wrapper(
      <div style={{ background: "#1a1400", border: "1px solid #f9a82544", borderRadius: 6, padding: "10px 12px" }}>
        <div style={{ color: "#f9a825", fontSize: 13, marginBottom: 6 }}>{t.vcs_token_required}</div>
        <button onClick={() => navigate("/settings")} style={{ ...smallBtnStyle, borderColor: "#f9a825", color: "#f9a825" }}>
          {t.vcs_btn_go_settings}
        </button>
      </div>
    );
  }

  if (result.type === "svn_not_installed") {
    return wrapper(
      <div style={{ background: "#2a0f0f", border: "1px solid #f8717144", borderRadius: 6, padding: "8px 12px", color: "#f87171", fontSize: 13 }}>
        {t.vcs_svn_required}
      </div>
    );
  }

  return null;
}

const smallBtnStyle: React.CSSProperties = {
  background: "transparent", border: "1px solid #3a3a3a", color: "#ccc",
  borderRadius: 4, padding: "3px 10px", cursor: "pointer", fontSize: 11,
};

// ── VcsLoopMessageBubble ──────────────────────────────────────────────────────

interface VcsLoopBubbleProps {
  message: VcsLoopMessage;
  writeMode: VcsWriteMode;
  onAction: (action: string, data: unknown) => void;
  onConfirmWrite: (intent: unknown) => void;
  onCancelWrite: () => void;
}

export function VcsLoopMessageBubble({ message, writeMode, onAction, onConfirmWrite, onCancelWrite }: VcsLoopBubbleProps) {
  const { t } = useLocale();
  const maxSteps = message.maxSteps ?? 0;
  if (message.kind === "user") {
    return (
      <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 8 }}>
        <div style={{ background: "#1e3a2e", border: "1px solid #34d39944", borderRadius: "8px 8px 2px 8px", padding: "8px 12px", maxWidth: "80%", color: "#e6e6e6", fontSize: 13 }}>
          {message.text}
        </div>
      </div>
    );
  }

  if (message.kind === "step-loading") {
    return (
      <div style={{ marginBottom: 8 }}>
        <div style={{ background: "#1a1a2e", border: "1px solid #3b3b6b", borderRadius: 8, padding: "10px 14px" }}>
          <div style={{ color: "#888", fontSize: 12, marginBottom: 4 }}>
            Step {message.stepNum}{maxSteps > 0 && maxSteps < 9999 ? `/${maxSteps}` : ""}
          </div>
          <div style={{ color: "#a78bfa", fontSize: 13 }}>
            <span className="vcs-view__step-spinner">⟳</span> {t.vcs_loop_status_running}
          </div>
        </div>
      </div>
    );
  }

  if (message.kind === "step") {
    return (
      <div style={{ marginBottom: 8 }}>
        <div style={{ background: "#1a1a2e", border: "1px solid #3b3b6b", borderRadius: 8, padding: "10px 14px" }}>
          <div style={{ color: "#888", fontSize: 12, marginBottom: 6 }}>
            Step {message.stepNum}{maxSteps > 0 && maxSteps < 9999 ? `/${maxSteps}` : ""}
          </div>
          {message.commandDisplay && (
            <div style={{ display: "flex", gap: 6, alignItems: "flex-start", marginBottom: 4 }}>
              <span style={{ fontSize: 13 }}>⚙️</span>
              <span style={{ color: "#a78bfa", fontSize: 12, fontFamily: "monospace" }}>{message.commandDisplay}</span>
            </div>
          )}
          {message.aiSummary && (
            <div style={{ display: "flex", gap: 6, alignItems: "flex-start", marginBottom: 8 }}>
              <span style={{ fontSize: 13 }}>💬</span>
              <span style={{ color: "#ccc", fontSize: 13 }}>{message.aiSummary}</span>
            </div>
          )}
          {message.result && (
            <div style={{ borderTop: "1px solid #2a2a3a", paddingTop: 8, marginTop: 4 }}>
              <VcsMessageBubble
                message={{ id: "inline", role: "assistant", text: "", result: message.result }}
                writeMode={writeMode}
                onAction={onAction}
                onConfirmWrite={onConfirmWrite}
                onCancelWrite={onCancelWrite}
              />
            </div>
          )}
        </div>
      </div>
    );
  }

  if (message.kind === "final-answer") {
    return (
      <div style={{ marginBottom: 8 }}>
        <div style={{ background: "#1e3a2e", border: "1px solid #34d39966", borderRadius: 8, padding: "12px 14px" }}>
          <div style={{ color: "#34d399", fontSize: 13, fontWeight: 600, marginBottom: 6 }}>{t.vcs_loop_status_done}</div>
          <div style={{ color: "#e6e6e6", fontSize: 13, whiteSpace: "pre-wrap" }}>{message.content}</div>
        </div>
      </div>
    );
  }

  if (message.kind === "step-limit-reached") {
    return (
      <div style={{ marginBottom: 8 }}>
        <div style={{ background: "#1a1400", border: "1px solid #f9a82544", borderRadius: 8, padding: "10px 14px" }}>
          <div style={{ color: "#f9a825", fontSize: 13 }}>⚠️ {message.content}</div>
        </div>
      </div>
    );
  }

  if (message.kind === "stopped") {
    return (
      <div style={{ marginBottom: 8 }}>
        <div style={{ background: "#1a1a1a", border: "1px solid #3a3a3a", borderRadius: 8, padding: "8px 14px" }}>
          <span style={{ color: "#888", fontSize: 13 }}>{t.vcs_loop_status_stopped}</span>
        </div>
      </div>
    );
  }

  if (message.kind === "error") {
    return (
      <div style={{ marginBottom: 8 }}>
        <div style={{ background: "#2a0f0f", border: "1px solid #f8717144", borderRadius: 8, padding: "8px 14px", color: "#f87171", fontSize: 13 }}>
          ✗ {message.content}
        </div>
      </div>
    );
  }

  return null;
}

