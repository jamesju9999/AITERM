import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useNavigate } from "react-router-dom";
import { useVcsCwd } from "../../hooks/useVcsCwd";
import { useVcsAgentLoop } from "../../hooks/useVcsAgentLoop";
import { VcsLoopMessageBubble } from "./VcsMessageBubble";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { pickFolder } from "../../ipc/vcs";
import "./VcsView.css";

const MANUAL_PATH_KEY = "aiterm-vcs-manual-path";

interface VcsViewProps {
  sessionId: string;
  isActive: boolean;
}

export function VcsView({ sessionId, isActive: _isActive }: VcsViewProps) {
  const navigate = useNavigate();

  // Manual path state — persisted in localStorage
  const [manualPath, setManualPath] = useState<string>(
    () => localStorage.getItem(MANUAL_PATH_KEY) ?? ""
  );
  const [isEditingPath, setIsEditingPath] = useState(false);
  const [pathInput, setPathInput] = useState("");

  const repoInfo = useVcsCwd(sessionId, manualPath || undefined);
  const { messages, isRunning, send, stop } = useVcsAgentLoop(sessionId, repoInfo);
  const [input, setInput] = useState("");
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState<string>("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      const def = list.find((p) => p.is_default);
      if (def) setSelectedProviderId(def.id);
    }).catch(console.error);
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
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
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
              title="選擇資料夾"
              style={iconBtnStyle}
            >📂</button>
            <button
              onClick={() => confirmPath(pathInput)}
              title="確認"
              style={{ ...iconBtnStyle, color: "#34d399" }}
            >✓</button>
            <button
              onClick={() => setIsEditingPath(false)}
              title="取消"
              style={{ ...iconBtnStyle, color: "#888" }}
            >✗</button>
          </>
        ) : (
          <>
            <span
              className="vcs-view__repo-root"
              title={manualPath ? `手動設定：${manualPath}（點擊編輯）` : "點擊設定路徑"}
              onClick={startEditing}
              style={{ cursor: "pointer", flex: 1 }}
            >
              {repoInfo?.root ?? (manualPath || "點擊設定路徑")}
              {manualPath && <span style={{ marginLeft: 4, color: "#555", fontSize: 10 }}>📌</span>}
            </span>
            <button onClick={handleBrowse} title="選擇資料夾" style={iconBtnStyle}>📂</button>
            {manualPath && (
              <button
                onClick={clearManualPath}
                title="清除手動路徑，回到自動偵測"
                style={{ ...iconBtnStyle, color: "#888" }}
              >✕</button>
            )}
            {repoInfo && <span className="vcs-view__write-mode">{writeMode}</span>}
          </>
        )}

        {/* Provider selector */}
        {!isEditingPath && (
          providers.length > 0 ? (
            <select
              value={selectedProviderId}
              onChange={(e) => setSelectedProviderId(e.target.value)}
              style={{ marginLeft: "auto", background: "#1a1a1a", border: "1px solid #333", color: "#ccc", borderRadius: 4, fontSize: 11, padding: "2px 6px", cursor: "pointer" }}
            >
              <option value="">預設</option>
              {providers.map((p) => (
                <option key={p.id} value={p.id}>{p.display_name}</option>
              ))}
            </select>
          ) : (
            <span style={{ marginLeft: "auto", fontSize: 11, color: "#555" }}>未設定 AI</span>
          )
        )}
      </div>

      {/* Messages */}
      <div className="vcs-view__messages">
        {!repoInfo ? (
          <div className="vcs-view__empty">
            {manualPath
              ? `找不到 VCS repo：${manualPath}`
              : "目前目錄不在版控 repo 中"}
          </div>
        ) : messages.length === 0 ? (
          <div className="vcs-view__empty">輸入問題來查詢版控資訊，例如：「最近 10 次 commit」</div>
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
            !repoInfo ? "未偵測到 VCS repo..." :
            isRunning ? "輸入以調整方向（Enter 送出）..." :
            "輸入問題，Enter 送出..."
          }
          rows={2}
          disabled={!repoInfo}
        />
        {isRunning ? (
          <button
            className="vcs-view__send-btn"
            onClick={stop}
            style={{ background: "#2a0f0f", borderColor: "#f87171", color: "#f87171" }}
          >
            停止
          </button>
        ) : (
          <button
            className="vcs-view__send-btn"
            onClick={handleSubmit}
            disabled={!repoInfo || input.trim() === ""}
          >
            送出
          </button>
        )}
      </div>

      {/* Link to settings if no connection */}
      {repoInfo && !repoInfo.connection_id && (
        <div style={{ padding: "6px 16px", borderTop: "1px solid #2a2a2a", background: "#111", fontSize: 11, color: "#666", display: "flex", gap: 6, alignItems: "center" }}>
          <span>本地模式</span>
          <button
            onClick={() => navigate("/settings")}
            style={{ background: "transparent", border: "none", color: "#34d399", cursor: "pointer", fontSize: 11, padding: 0, textDecoration: "underline" }}
          >
            前往設定新增連線
          </button>
        </div>
      )}
    </div>
  );
}

const iconBtnStyle: React.CSSProperties = {
  background: "transparent", border: "none", cursor: "pointer",
  fontSize: 14, padding: "0 4px", color: "#ccc", lineHeight: 1,
};
