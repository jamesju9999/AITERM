import { useState, useRef, useEffect, useCallback, type KeyboardEvent } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { isImeComposing } from "../../lib/imeComposing";
import { pickFolder } from "../../ipc/vcs";
import { writeTextFile } from "../../ipc/fs";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { getConfig, type SubmitShortcut } from "../../ipc/config";
import { useCodeAssistant } from "../../hooks/useCodeAssistant";
import { useLocale } from "../../contexts/LocaleContext";
import { ToolCallCard } from "./ToolCallCard";
import { MarkdownText } from "../../lib/markdown";
import { ModelPickerButton } from "../ModelPickerButton";
import { CloseConfirmDialog } from "../CloseConfirmDialog";
import { ArtifactPanelProvider } from "../../contexts/ArtifactPanelContext";
import { ArtifactSplit } from "../ArtifactPanel/ArtifactSplit";
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
  tabId?: string;
  registerCloseGuard?: (tabId: string, guard: () => Promise<boolean>) => void;
  unregisterCloseGuard?: (tabId: string) => void;
}

export function CodeAssistantView(props: Props) {
  return (
    <ArtifactPanelProvider>
      <CodeAssistantViewInner {...props} />
    </ArtifactPanelProvider>
  );
}

function CodeAssistantViewInner({
  isActive,
  tabId,
  registerCloseGuard,
  unregisterCloseGuard,
}: Props) {
  const { t } = useLocale();
  const [projectRoot, setProjectRoot] = useState(loadSavedRoot);
  const [pendingRoot, setPendingRoot] = useState<string | null>(null);
  const [input, setInput] = useState("");
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [submitShortcut, setSubmitShortcut] = useState<SubmitShortcut>("enter");
  const submitShortcutRef = useRef<SubmitShortcut>("enter");
  submitShortcutRef.current = submitShortcut;
  const messagesContainerRef = useRef<HTMLDivElement>(null);
  // True while the user is parked at (or near) the bottom — only then do we
  // auto-follow new content. Flips to false the moment they scroll up to read.
  const shouldAutoScrollRef = useRef(true);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const { messages, isStreaming, error, isFallbackMode, tokenCount, tokenLimit, send, clear } = useCodeAssistant();

  // 關閉確認：ref 是必要的，不是風格選擇。guard 只在 tabId 變動時重新註冊，
  // 若直接閉包捕捉 messages/isStreaming，之後談的每一輪它都看不到，
  // 會在沒有任何錯誤訊號的情況下直接放行——功能等於靜默失效。
  const hasMessagesRef = useRef(false);
  hasMessagesRef.current = messages.length > 0;
  const isStreamingRef = useRef(false);
  isStreamingRef.current = isStreaming;

  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  const closeResolveRef = useRef<((canClose: boolean) => void) | null>(null);

  const handleCloseConfirm = useCallback((canClose: boolean) => {
    setShowCloseConfirm(false);
    closeResolveRef.current?.(canClose);
    closeResolveRef.current = null;
  }, []);

  useEffect(() => {
    if (!tabId || !registerCloseGuard) return;
    registerCloseGuard(tabId, () => {
      // 全新、沒談過話的分頁沒有東西可失去，不要打擾使用者。
      if (!isStreamingRef.current && !hasMessagesRef.current) {
        return Promise.resolve(true);
      }
      return new Promise<boolean>((resolve) => {
        closeResolveRef.current = resolve;
        setShowCloseConfirm(true);
      });
    });
    return () => { unregisterCloseGuard?.(tabId); };
  }, [tabId, registerCloseGuard, unregisterCloseGuard]);

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

  // Auto-scroll to the newest content while generating — but only when the user
  // is already at the bottom, and keep following as content keeps GROWING. The
  // reply's Mermaid diagram and markdown render asynchronously (their height
  // lands after `messages` updates), so scrolling on `messages` alone left the
  // view stranded in the middle once the diagram appeared. A ResizeObserver on
  // the message elements (plus a MutationObserver for newly-added ones) catches
  // that late height and re-pins to the bottom.
  useEffect(() => {
    const container = messagesContainerRef.current;
    if (!container) return;

    const scrollToBottom = () => {
      if (shouldAutoScrollRef.current) container.scrollTop = container.scrollHeight;
    };

    const ro = new ResizeObserver(scrollToBottom);
    const observeChildren = () => {
      for (const child of Array.from(container.children)) ro.observe(child);
    };
    observeChildren();

    const mo = new MutationObserver(() => {
      observeChildren();
      scrollToBottom();
    });
    mo.observe(container, { childList: true, subtree: true });

    return () => {
      ro.disconnect();
      mo.disconnect();
    };
  }, []);

  // Track whether the user is parked at the bottom (auto-follow) or has scrolled
  // up to read (stop yanking them down).
  const handleMessagesScroll = useCallback(() => {
    const el = messagesContainerRef.current;
    if (!el) return;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    shouldAutoScrollRef.current = distanceFromBottom < 80;
  }, []);

  // On a brand-new user turn (the user just sent), re-pin to the bottom and
  // resume following — even if they had scrolled up while reading the last
  // answer. Keyed on the user-message count so assistant/tool messages arriving
  // mid-generation don't yank a user who deliberately scrolled up.
  const userMsgCountRef = useRef(0);
  useEffect(() => {
    const userCount = messages.reduce((n, m) => (m.role === "user" ? n + 1 : n), 0);
    if (userCount > userMsgCountRef.current) {
      shouldAutoScrollRef.current = true;
      const container = messagesContainerRef.current;
      if (container) container.scrollTop = container.scrollHeight;
    }
    userMsgCountRef.current = userCount;
  }, [messages]);

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
    if (isImeComposing(e)) return;
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

  // 兩個 return 分支都要掛同一個確認框（沒選專案目錄時一樣可能已經有對話），
  // 所以算一次就好——分開寫兩份的話，日後改了其中一份會靜默地不同步。
  const closeConfirmDialog = showCloseConfirm && (
    <CloseConfirmDialog
      title={isStreaming ? t.ca_close_title_streaming : t.ca_close_title_dirty}
      body={isStreaming ? t.ca_close_body_streaming : t.ca_close_body_dirty}
      confirmLabel={t.ca_close_discard}
      cancelLabel={t.ca_close_cancel}
      onConfirm={() => handleCloseConfirm(true)}
      onCancel={() => handleCloseConfirm(false)}
    />
  );

  if (!projectRoot) {
    return (
      <div className="ca-view">
        {closeConfirmDialog}
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
      {closeConfirmDialog}
      <ArtifactSplit>
      {/* Messages area */}
      <div className="ca-messages" ref={messagesContainerRef} onScroll={handleMessagesScroll}>
        {messages.length === 0 && (
          <div className="ca-hint-center">
            <div className="ca-hint-icon">
              <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="16 18 22 12 16 6" />
                <polyline points="8 6 2 12 8 18" />
              </svg>
            </div>
            <div className="ca-hint-title">{t.ca_hint_title}</div>
            <div className="ca-hint-desc">{t.ca_hint_desc(projectRoot)}</div>
            <div className="ca-hint-examples">
              {t.ca_hint_examples.map((ex) => (
                <button key={ex} className="ca-hint-chip" onClick={() => setInput(ex)}>
                  {ex}
                </button>
              ))}
            </div>
          </div>
        )}
        {messages.map((msg, i) => {
          const isDone = msg.role === "assistant" && !msg.streaming;
          const readPaths = isDone
            ? [...new Set(
                (msg.toolCalls ?? [])
                  .filter((tc) =>
                    (tc.tool === "read_file" || tc.tool === "read_file_lines") &&
                    tc.result !== undefined &&
                    !tc.result.content.startsWith("Error:")
                  )
                  .map((tc) => String(tc.args.path ?? ""))
                  .filter(Boolean)
              )]
            : [];
          const searchedPaths = isDone
            ? [...new Set(
                (msg.toolCalls ?? [])
                  .filter((tc) =>
                    tc.tool === "search_in_files" &&
                    tc.result !== undefined &&
                    !tc.result.content.startsWith("Error:")
                  )
                  .map((tc) => {
                    const q = String(tc.args.query ?? "");
                    const p = String(tc.args.path ?? "");
                    return p ? `${q} (${p})` : q;
                  })
                  .filter(Boolean)
              )]
            : [];

          return (
            <div key={i} className={`ca-msg ca-msg--${msg.role}`}>
              {msg.role === "assistant" && (msg.toolCalls ?? []).map((tc) => (
                <ToolCallCard key={tc.callId} toolCall={tc} />
              ))}
              {msg.role === "assistant" && (msg.checkpoints ?? []).map((n) => (
                <div key={`checkpoint-${n}`} className="ca-checkpoint-badge">
                  {t.ca_checkpoint_notice(n)}
                </div>
              ))}
              {(msg.content || msg.streaming) && (
                <div className="ca-msg__bubble">
                  {msg.role === "assistant" && msg.streaming && !msg.content ? (
                    <span className="ca-thinking-indicator"><span /><span /><span /></span>
                  ) : (
                    <>
                      {msg.role === "assistant" ? (
                        <MarkdownText text={msg.content} />
                      ) : (
                        msg.content
                      )}
                      {msg.streaming && <span className="ca-streaming-cursor" />}
                    </>
                  )}
                </div>
              )}
              {(readPaths.length > 0 || searchedPaths.length > 0) && (
                <div className="ca-files-accessed">
                  {readPaths.length > 0 && (
                    <div className="ca-files-read">
                      <span className="ca-files-read__label">{t.ca_files_read_label}</span>
                      {readPaths.map((p) => (
                        <span key={p} className="ca-files-read__path" title={p}>
                          {p.split("/").pop()}
                        </span>
                      ))}
                    </div>
                  )}
                  {searchedPaths.length > 0 && (
                    <div className="ca-files-searched">
                      <span className="ca-files-searched__label">{t.ca_files_searched_label}</span>
                      {searchedPaths.map((s) => (
                        <span key={s} className="ca-files-searched__query" title={s}>
                          {s}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          );
        })}
        {error && <div className="ca-error">{error}</div>}
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
        <ModelPickerButton
          providers={providers}
          selectedId={selectedProviderId}
          onChange={setSelectedProviderId}
        />
        {isStreaming && tokenCount > 0 && (
          <span className="ca-token-count" title={`估算 token 用量（上限 ${tokenLimit.toLocaleString()}）`}>
            {tokenCount.toLocaleString()} / {tokenLimit.toLocaleString()}
          </span>
        )}
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
      </ArtifactSplit>
    </div>
  );
}
