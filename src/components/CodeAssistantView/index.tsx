import { useState, useRef, useEffect, useCallback, type KeyboardEvent } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { pickFolder } from "../../ipc/vcs";
import { writeTextFile } from "../../ipc/fs";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { getConfig, type SubmitShortcut } from "../../ipc/config";
import { useCodeAssistant } from "../../hooks/useCodeAssistant";
import { useLocale } from "../../contexts/LocaleContext";
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
  const { t } = useLocale();
  const [projectRoot, setProjectRoot] = useState(loadSavedRoot);
  const [pendingRoot, setPendingRoot] = useState<string | null>(null);
  const [input, setInput] = useState("");
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [submitShortcut, setSubmitShortcut] = useState<SubmitShortcut>("enter");
  const submitShortcutRef = useRef<SubmitShortcut>("enter");
  submitShortcutRef.current = submitShortcut;
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const { messages, isStreaming, error, isFallbackMode, send, clear } = useCodeAssistant();

  // Load providers once on mount
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

  // Re-read submit shortcut every time this tab becomes active, so changes in
  // Settings are picked up without needing to close and reopen the tab.
  useEffect(() => {
    if (isActive) {
      getConfig().then((cfg) => setSubmitShortcut(cfg.submit_shortcut ?? "enter")).catch(() => {});
    }
  }, [isActive]);

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

  const shortcutLabel = submitShortcut === "shift-enter" ? "Shift+Enter" : submitShortcut === "ctrl-enter" ? "Ctrl+Enter" : "Enter";

  const handleKeyDown = useCallback((e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key !== "Enter") return;
    const sc = submitShortcutRef.current;
    const ok = (sc === "enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) ||
               (sc === "shift-enter" && e.shiftKey && !e.ctrlKey) ||
               (sc === "ctrl-enter" && (e.ctrlKey || e.metaKey) && !e.shiftKey);
    if (ok) { e.preventDefault(); handleSend(); }
  }, [handleSend]);

  const handleExport = useCallback(async () => {
    if (messages.length === 0) return;
    const path = await save({
      defaultPath: "conversation.md",
      filters: [{ name: "Markdown", extensions: ["md"] }],
    });
    if (!path) return;
    const lines: string[] = [`# 程式庫問答紀錄\n`, `**專案目錄：** \`${projectRoot}\`\n`];
    for (const msg of messages) {
      if (msg.role === "user") {
        lines.push(`\n---\n\n**問：** ${msg.content}\n`);
      } else {
        if ((msg.toolCalls ?? []).length > 0) {
          lines.push("\n**工具調用：**\n");
          for (const tc of msg.toolCalls!) {
            lines.push(`- \`${tc.tool}\`（${JSON.stringify(tc.args)}）`);
          }
          lines.push("");
        }
        if (msg.content) {
          lines.push(`\n**答：**\n\n${msg.content}\n`);
        }
      }
    }
    await writeTextFile(path, lines.join("\n"));
  }, [messages, projectRoot]);

  if (!projectRoot) {
    return (
      <div className="ca-view">
        <div className="ca-empty">
          <div className="ca-empty__icon">📂</div>
          <div className="ca-empty__title">{t.ca_empty_title}</div>
          <div className="ca-empty__desc">{t.ca_empty_desc}</div>
          <button className="aiterm-btn aiterm-btn--primary" onClick={handlePickFolder}>
            {t.ca_pick_folder}
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="ca-view">
      {/* Messages area */}
      <div className="ca-messages">
        {messages.length === 0 && (
          <div className="ca-hint">{t.ca_hint(projectRoot)}</div>
        )}
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

      {/* Fallback mode banner */}
      {isFallbackMode && (
        <div className="ca-fallback-banner">{t.ca_fallback_banner}</div>
      )}

      {/* Directory change confirmation bar */}
      {pendingRoot && (
        <div className="ca-dir-confirm">
          <span className="ca-dir-confirm__path">{t.ca_dir_switch(pendingRoot)}</span>
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => handleConfirmDir(false)}>{t.ca_dir_continue}</button>
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => handleConfirmDir(true)}>{t.ca_dir_new_chat}</button>
          <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => setPendingRoot(null)}>{t.ca_cancel}</button>
        </div>
      )}

      {/* Toolbar: path + model selector + actions */}
      <div className="ca-toolbar">
        <span className="ca-toolbar__label">{t.ca_label_dir}</span>
        <button
          className="ca-toolbar__path"
          title={projectRoot}
          onClick={handlePickFolder}
        >
          {projectRoot}
        </button>
        <div style={{ width: 1, background: "#2a2a2a", alignSelf: "stretch", margin: "0 4px" }} />
        <span className="ca-toolbar__label">{t.ca_label_model}</span>
        <select
          className="ca-provider-select"
          value={selectedProviderId}
          onChange={(e) => setSelectedProviderId(e.target.value)}
        >
          {providers.map((p) => (
            <option key={p.id} value={p.id}>
              {p.display_name} ({p.model}){p.is_default ? " ★" : ""}
            </option>
          ))}
          {providers.length === 0 && <option value="">{t.ca_no_provider}</option>}
        </select>
        {messages.length > 0 && (
          <>
            <button
              className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
              onClick={handleExport}
            >
              {t.ca_export}
            </button>
            <button
              className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
              onClick={clear}
            >
              {t.ca_clear}
            </button>
          </>
        )}
      </div>

      {/* Input pill */}
      <div className="ca-input-row">
        <div className="aiterm-input-pill-container" style={{
          display: "flex", alignItems: "center",
          background: "rgba(255, 255, 255, 0.03)",
          border: "1px solid rgba(255, 255, 255, 0.08)",
          borderRadius: 20, padding: "4px 8px", flex: 1, gap: 6,
        }}>
          <textarea
            ref={textareaRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={t.ca_input_placeholder(shortcutLabel)}
            rows={1}
            disabled={isStreaming}
            style={{
              flex: 1, background: "transparent", border: "none",
              color: "var(--text-primary)", padding: "4px 6px", fontSize: 13,
              resize: "none", outline: "none", fontFamily: "inherit",
              height: 24, lineHeight: "24px", overflowY: "hidden",
            }}
          />
          <button
            onClick={handleSend}
            disabled={isStreaming || !input.trim()}
            className="aiterm-btn aiterm-btn--primary aiterm-btn--icon"
            title="送出 (Enter)"
          >
            ▲
          </button>
        </div>
      </div>
    </div>
  );
}
