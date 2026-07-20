import { useState, useRef, useEffect, useCallback, type KeyboardEvent } from "react";
import { pickFolder } from "../../ipc/vcs";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { useCodeAssistant } from "../../hooks/useCodeAssistant";
import { ToolCallCard } from "./ToolCallCard";
import { MarkdownText } from "../../lib/markdown";
import "./styles.css";

const STORAGE_KEY = "aiterm-code-assistant-root";

function loadSavedRoot(): string {
  try { return localStorage.getItem(STORAGE_KEY) ?? ""; } catch { return ""; }
}
function saveRoot(path: string) {
  try { localStorage.setItem(STORAGE_KEY, path); } catch { /* ignore */ }
}

interface Props {
  isActive: boolean;
}

export function CodeAssistantView({ isActive }: Props) {
  const [projectRoot, setProjectRoot] = useState(loadSavedRoot);
  const [pendingRoot, setPendingRoot] = useState<string | null>(null);
  const [input, setInput] = useState("");
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const { messages, isStreaming, error, isFallbackMode, send, clear } = useCodeAssistant();

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      if (list.length > 0 && !selectedProviderId) {
        const def = list.find((p) => p.is_default) ?? list[0];
        setSelectedProviderId(def.id);
      }
    }).catch(() => {});
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (isActive) {
      messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [messages, isActive]);

  const handlePickFolder = useCallback(async () => {
    const folder = await pickFolder();
    if (!folder) return;
    if (messages.length > 0) {
      setPendingRoot(folder);
    } else {
      setProjectRoot(folder);
      saveRoot(folder);
    }
  }, [messages.length]);

  const handleConfirmDir = useCallback((newChat: boolean) => {
    if (!pendingRoot) return;
    setProjectRoot(pendingRoot);
    saveRoot(pendingRoot);
    setPendingRoot(null);
    if (newChat) clear();
  }, [pendingRoot, clear]);

  const handleSend = useCallback(() => {
    if (!input.trim() || isStreaming || !projectRoot) return;
    const text = input;
    setInput("");
    void send(text, projectRoot, selectedProviderId || undefined);
  }, [input, isStreaming, projectRoot, selectedProviderId, send]);

  const handleKeyDown = useCallback((e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }, [handleSend]);

  if (!projectRoot) {
    return (
      <div className="ca-view">
        <div className="ca-empty">
          <div className="ca-empty__icon">📂</div>
          <div className="ca-empty__title">選擇專案目錄</div>
          <div className="ca-empty__desc">選定目錄後即可對程式庫提問</div>
          <button className="aiterm-btn aiterm-btn--primary" onClick={handlePickFolder}>
            選擇目錄
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="ca-view">
      {/* Header: path + actions */}
      <div className="ca-header">
        <span className="ca-header__path" title={projectRoot}>📁 {projectRoot}</span>
        <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={handlePickFolder}>更換目錄</button>
        <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={clear}>清除</button>
      </div>

      {/* Toolbar: model selector */}
      <div className="ca-toolbar">
        <span className="ca-toolbar__label">模型</span>
        {providers.length > 0 ? (
          <select
            className="ca-provider-select"
            value={selectedProviderId}
            onChange={(e) => setSelectedProviderId(e.target.value)}
          >
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.display_name}{p.is_default ? " ★" : ""}
              </option>
            ))}
          </select>
        ) : (
          <span className="ca-toolbar__no-provider">未設定 Provider — 請至設定頁面新增</span>
        )}
      </div>

      {/* Directory change confirmation bar */}
      {pendingRoot && (
        <div className="ca-dir-confirm">
          <span className="ca-dir-confirm__path">已選擇 {pendingRoot}</span>
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => handleConfirmDir(false)}>繼續對話</button>
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => handleConfirmDir(true)}>開新對話</button>
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => setPendingRoot(null)}>取消</button>
        </div>
      )}

      {/* Fallback mode banner */}
      {isFallbackMode && (
        <div className="ca-fallback-banner">
          ⚠ 此 provider 不支援工具調用，已切換為兩段式模式
        </div>
      )}

      {/* Messages */}
      <div className="ca-messages">
        {messages.map((msg, i) => (
          <div key={i} className={`ca-msg ca-msg--${msg.role}`}>
            {msg.role === "assistant" && (msg.toolCalls ?? []).map((tc) => (
              <ToolCallCard key={tc.callId} toolCall={tc} />
            ))}
            {(msg.content || msg.streaming) && (
              <div className="ca-msg__bubble">
                {msg.role === "assistant" ? (
                  <MarkdownText text={msg.content} />
                ) : (
                  msg.content
                )}
                {msg.streaming && <span className="ca-streaming-cursor" />}
              </div>
            )}
          </div>
        ))}
        {error && <div className="ca-error">{error}</div>}
        <div ref={messagesEndRef} />
      </div>

      {/* Input bar */}
      <div className="ca-input-bar">
        <textarea
          className="ca-input-bar__textarea"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="問關於這個專案的任何問題... (Enter 送出，Shift+Enter 換行)"
          rows={1}
          disabled={isStreaming}
        />
        <button
          className="aiterm-btn aiterm-btn--primary aiterm-btn--sm ca-input-bar__send"
          onClick={handleSend}
          disabled={isStreaming || !input.trim()}
        >
          {isStreaming ? "..." : "送出"}
        </button>
      </div>
    </div>
  );
}
