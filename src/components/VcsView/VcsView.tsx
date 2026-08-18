import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useNavigate } from "react-router-dom";
import { useVcsCwd } from "../../hooks/useVcsCwd";
import { useVcsAgentLoop } from "../../hooks/useVcsAgentLoop";
import { useTeamFeatures } from "../../hooks/useTeamFeatures";
import { VcsLoopMessageBubble } from "./VcsMessageBubble";
import { TeamPanel } from "./TeamPanel";
import { StartFeatureDialog } from "./StartFeatureDialog";
import { FinishFeatureReview } from "./FinishFeatureReview";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { getConfig, type SubmitShortcut } from "../../ipc/config";
import { pickFolder, type ActiveFeature, type VcsRepoInfo } from "../../ipc/vcs";
import { useLocale } from "../../contexts/LocaleContext";
import { ModelPickerButton } from "../ModelPickerButton";
import "./VcsView.css";

const MANUAL_PATH_KEY = "aiterm-vcs-manual-path";

interface VcsViewProps {
  sessionId: string;
  isActive: boolean;
}

export function VcsView({ sessionId, isActive: _isActive }: VcsViewProps) {
  const { t } = useLocale();
  const navigate = useNavigate();

  // Manual path state — persisted in localStorage
  const [manualPath, setManualPath] = useState<string>(
    () => localStorage.getItem(MANUAL_PATH_KEY) ?? ""
  );
  const [isEditingPath, setIsEditingPath] = useState(false);
  const [pathInput, setPathInput] = useState("");

  const repoInfo = useVcsCwd(sessionId, manualPath || undefined);
  const { messages, isRunning, send, stop } = useVcsAgentLoop(sessionId, repoInfo);
  const isGitRepo = repoInfo?.vcs_type === "git";
  const { features, loading: featuresLoading, error: featuresError, refresh: refreshFeatures } = useTeamFeatures(isGitRepo ? repoInfo : null);
  // 開啟對話框當下就把 repoInfo 快照下來，避免 useVcsCwd 的輪詢在對話框開著時把它換掉或設成 null。
  // 用單一 state 存兩種對話框，天然保證同時最多只有一個開著。
  const [activeDialog, setActiveDialog] = useState<
    { kind: "start"; repo: VcsRepoInfo } | { kind: "finish"; feature: ActiveFeature; repo: VcsRepoInfo } | null
  >(null);
  const [input, setInput] = useState("");
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState<string>("");
  const [submitShortcut, setSubmitShortcut] = useState<SubmitShortcut>("enter");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      const def = list.find((p) => p.is_default);
      if (def) setSelectedProviderId(def.id);
    }).catch(console.error);
    getConfig().then((cfg) => setSubmitShortcut(cfg.submit_shortcut ?? "enter")).catch(() => {});
  }, []);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const confirmPath = (path: string) => {
    const trimmed = path.trim();
    if (trimmed) {
      localStorage.setItem(MANUAL_PATH_KEY, trimmed);
      setManualPath(trimmed);
    }
    setIsEditingPath(false);
  };

  const clearManualPath = () => {
    localStorage.removeItem(MANUAL_PATH_KEY);
    setManualPath("");
    setIsEditingPath(false);
  };

  const startEditing = () => {
    setPathInput(manualPath || repoInfo?.root || "");
    setIsEditingPath(true);
  };

  const handleBrowse = async () => {
    const picked = await pickFolder();
    if (picked) {
      confirmPath(picked);
    }
  };

  const handlePathKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") { e.preventDefault(); confirmPath(pathInput); }
    if (e.key === "Escape") { e.preventDefault(); setIsEditingPath(false); }
  };

  const handleKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter") {
      const ok = (submitShortcut === "enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) ||
                 (submitShortcut === "shift-enter" && e.shiftKey && !e.ctrlKey) ||
                 (submitShortcut === "ctrl-enter" && (e.ctrlKey || e.metaKey) && !e.shiftKey);
      if (ok) { e.preventDefault(); handleSubmit(); }
    }
  };

  const handleSubmit = () => {
    const text = input.trim();
    if (!text || !repoInfo) return;
    setInput("");
    send(text, selectedProviderId || null);
  };

  const handleAction = (action: string, data: unknown) => {
    if (action === "view_diff") {
      send(`show diff for ${(data as { revision: string }).revision}`, selectedProviderId || null);
    } else if (action === "revert") {
      send(`revert commit ${(data as { revision: string }).revision}`, selectedProviderId || null);
    } else if (action === "view_pr_comments") {
      send(`show comments for PR #${(data as { number: number }).number}`, selectedProviderId || null);
    } else if (action === "merge_pr") {
      send(`merge PR #${(data as { number: number }).number}`, selectedProviderId || null);
    }
  };

  const handleConfirmWrite = (intent: unknown) => {
    send(`confirm_write:${JSON.stringify(intent)}`, selectedProviderId || null);
  };

  const writeMode = repoInfo?.connection_id ? "guarded" : "read_only";

  return (
    <div className="vcs-view">
      {/* Header */}
      <div className="vcs-view__header">
        {/* VCS badge */}
        {repoInfo && (
          <span className={`vcs-view__badge${repoInfo.vcs_type === "svn" ? " vcs-view__badge--svn" : ""}`}>
            {repoInfo.vcs_type.toUpperCase()}
          </span>
        )}

        {/* Path display / edit area */}
        {isEditingPath ? (
          <>
            <input
              type="text"
              value={pathInput}
              onChange={(e) => setPathInput(e.target.value)}
              onKeyDown={handlePathKeyDown}
              autoFocus
              style={{
                flex: 1, background: "#111", border: "1px solid #34d399", color: "#e6e6e6",
                borderRadius: 4, padding: "2px 8px", fontSize: 12, fontFamily: "monospace",
              }}
              placeholder="/path/to/repo"
            />
            <button
              onClick={handleBrowse}
              title={t.vcs_btn_browse}
              style={iconBtnStyle}
            >📂</button>
            <button
              onClick={() => confirmPath(pathInput)}
              title={t.vcs_btn_confirm}
              style={{ ...iconBtnStyle, color: "#34d399" }}
            >✓</button>
            <button
              onClick={() => setIsEditingPath(false)}
              title={t.vcs_btn_cancel}
              style={{ ...iconBtnStyle, color: "#888" }}
            >✗</button>
          </>
        ) : (
          <>
            <span
              className="vcs-view__repo-root"
              title={manualPath ? t.vcs_path_tooltip_manual(manualPath) : t.vcs_path_tooltip_empty}
              onClick={startEditing}
              style={{ cursor: "pointer", flex: 1 }}
            >
              {repoInfo?.root ?? (manualPath || t.vcs_path_tooltip_empty)}
              {manualPath && <span style={{ marginLeft: 4, color: "#555", fontSize: 10 }}>📌</span>}
            </span>
            <button onClick={handleBrowse} title={t.vcs_btn_browse} style={iconBtnStyle}>📂</button>
            {manualPath && (
              <button
                onClick={clearManualPath}
                title={t.vcs_clear_path_tooltip}
                style={{ ...iconBtnStyle, color: "#888" }}
              >✕</button>
            )}
            {repoInfo && <span className="vcs-view__write-mode">{writeMode}</span>}
          </>
        )}

        {/* Provider selector */}
        {!isEditingPath && (
          providers.length > 0 ? (
            <div style={{ marginLeft: "auto" }}>
              <ModelPickerButton
                providers={providers}
                selectedId={selectedProviderId}
                onChange={setSelectedProviderId}
              />
            </div>
          ) : (
            <span style={{ marginLeft: "auto", fontSize: 11, color: "#555" }}>{t.vcs_no_ai}</span>
          )
        )}
      </div>

      {/* Team panel */}
      {repoInfo && isGitRepo && (
        <>
          <TeamPanel
            features={features}
            loading={featuresLoading}
            onRefresh={refreshFeatures}
            onStartFeature={() => setActiveDialog({ kind: "start", repo: repoInfo })}
            onFinishFeature={(f) => setActiveDialog({ kind: "finish", feature: f, repo: repoInfo })}
          />
          {featuresError && (
            <div style={{ padding: "4px 16px", fontSize: 11, color: "#f87171" }}>
              {featuresError.startsWith("no_token") ? t.vcs_no_token : featuresError}
            </div>
          )}
        </>
      )}

      {/* Messages */}
      <div className="vcs-view__messages">
        {!repoInfo ? (
          <div className="vcs-view__empty">
            {manualPath
              ? t.vcs_repo_not_found(manualPath)
              : t.vcs_no_repo}
          </div>
        ) : messages.length === 0 ? (
          <div className="vcs-view__empty">{t.vcs_welcome_hint}</div>
        ) : (
          messages.map((msg) => (
            <VcsLoopMessageBubble
              key={msg.id}
              message={msg}
              writeMode={writeMode}
              onAction={handleAction}
              onConfirmWrite={handleConfirmWrite}
              onCancelWrite={() => {}}
            />
          ))
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* Input area */}
      <div className="vcs-view__input-area">
        <textarea
          ref={textareaRef}
          className="vcs-view__input"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={
            !repoInfo ? t.vcs_placeholder_no_repo :
            isRunning ? t.vcs_placeholder_running :
            t.vcs_placeholder_idle
          }
          rows={2}
          disabled={!repoInfo}
        />
        {isRunning ? (
          <button
            className="vcs-view__send-btn aiterm-btn aiterm-btn--danger-solid aiterm-btn--icon"
            onClick={stop}
            title={t.vcs_btn_stop}
          >
            ■
          </button>
        ) : (
          <button
            className="vcs-view__send-btn aiterm-btn aiterm-btn--primary aiterm-btn--icon"
            onClick={handleSubmit}
            disabled={!repoInfo || input.trim() === ""}
            title={t.vcs_btn_send}
          >
            ▲
          </button>
        )}
      </div>

      {/* Link to settings if no connection */}
      {repoInfo && !repoInfo.connection_id && (
        <div style={{ padding: "6px 16px", borderTop: "1px solid #2a2a2a", background: "#111", fontSize: 11, color: "#666", display: "flex", gap: 6, alignItems: "center" }}>
          <span>{t.vcs_local_mode_label}</span>
          <button
            onClick={() => navigate("/settings")}
            style={{ background: "transparent", border: "none", color: "#34d399", cursor: "pointer", fontSize: 11, padding: 0, textDecoration: "underline" }}
          >
            {t.vcs_add_conn_hint}
          </button>
        </div>
      )}

      {activeDialog?.kind === "start" && (
        <StartFeatureDialog
          repoInfo={activeDialog.repo}
          baseBranch="main"
          onStarted={() => { setActiveDialog(null); void refreshFeatures(); }}
          onClose={() => setActiveDialog(null)}
        />
      )}

      {activeDialog?.kind === "finish" && (
        <FinishFeatureReview
          repoInfo={activeDialog.repo}
          feature={activeDialog.feature}
          baseBranch="main"
          onSubmittedForReview={refreshFeatures}
          onMerged={() => { setActiveDialog(null); void refreshFeatures(); }}
          onClose={() => setActiveDialog(null)}
        />
      )}
    </div>
  );
}

const iconBtnStyle: React.CSSProperties = {
  background: "transparent", border: "none", cursor: "pointer",
  fontSize: 14, padding: "0 4px", color: "#ccc", lineHeight: 1,
};

