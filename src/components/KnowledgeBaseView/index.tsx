import { useState, useRef, useEffect, useCallback, type KeyboardEvent } from "react";
import { useNavigate } from "react-router-dom";
import { isImeComposing } from "../../lib/imeComposing";
import { getConfig, type SubmitShortcut } from "../../ipc/config";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { kbOpenDocument } from "../../ipc/knowledgeBase";
import { useNotebooks } from "../../hooks/useNotebooks";
import { useKnowledgeBaseChat } from "../../hooks/useKnowledgeBaseChat";
import { useLocale } from "../../contexts/LocaleContext";
import { ToolCallCard } from "../CodeAssistantView/ToolCallCard";
import { MarkdownText } from "../../lib/markdown";
import { ModelPickerButton } from "../ModelPickerButton";
import { NotebookSidebar } from "./NotebookSidebar";
import { NotebookCreateDialog } from "./NotebookCreateDialog";
import { ChatHistorySidebar } from "./ChatHistorySidebar";
import { usePythonEnvGate } from "../PythonEnv/usePythonEnvGate";
import { PythonEnvGate } from "../PythonEnv/PythonEnvGate";
import { ArtifactPanelProvider } from "../../contexts/ArtifactPanelContext";
import { ArtifactSplit } from "../ArtifactPanel/ArtifactSplit";
// 重用 Code Assistant 的聊天氣泡/工具卡片樣式（ca-msg、ca-hint-*、ca-toolbar 等）。
// 這裡明確 import，不依賴「CodeAssistantView 剛好也被載入過」這種隱性順序。
import "../CodeAssistantView/styles.css";
import "./styles.css";

const STORAGE_KEY = "aiterm-knowledge-base-active-notebook";

function loadSavedNotebookId(): string | null {
  try { return localStorage.getItem(STORAGE_KEY); } catch { return null; }
}
function saveNotebookId(id: string | null) {
  try {
    if (id) localStorage.setItem(STORAGE_KEY, id);
    else localStorage.removeItem(STORAGE_KEY);
  } catch { /* ignore */ }
}

const HISTORY_WIDTH_KEY = "aiterm-knowledge-base-history-width";

function loadSavedHistoryWidth(): number {
  try {
    const raw = localStorage.getItem(HISTORY_WIDTH_KEY);
    const n = raw ? parseInt(raw, 10) : NaN;
    return Number.isFinite(n) ? n : 280;
  } catch { return 280; }
}
function saveHistoryWidth(width: number) {
  try { localStorage.setItem(HISTORY_WIDTH_KEY, String(width)); } catch { /* ignore */ }
}

const HISTORY_COLLAPSED_KEY = "aiterm-knowledge-base-history-collapsed";
const HISTORY_COLLAPSED_WIDTH = 32;

function loadSavedHistoryCollapsed(): boolean {
  try { return localStorage.getItem(HISTORY_COLLAPSED_KEY) === "1"; } catch { return false; }
}
function saveHistoryCollapsed(collapsed: boolean) {
  try { localStorage.setItem(HISTORY_COLLAPSED_KEY, collapsed ? "1" : "0"); } catch { /* ignore */ }
}

// search_documents 的結果格式："[1] report.pdf — 第一章 (score 0.85)\n<內容>"
// 注意：第一個 capture group（rel_path）刻意用「貪婪」比對（.+ 而非 .+?），
// 且整體錨定到行尾（\)$）。中文檔名很常見 em dash（例如「會議記錄 — 2026.pdf」），
// 若 rel_path 本身包含 " — "，非貪婪比對只會切到「第一個」— 導致路徑被截斷、
// location 吃到路徑的其餘部分。貪婪比對會反向從最長開始回溯，
// 正確地切到「最後一個」— 也就是路徑與 location 之間真正的分隔點，
// 只有在 location 本身也包含 em dash 時才會誤判（較少見：標題文字含 em dash）。
const SOURCE_LINE_RE = /^\[\d+\]\s+(.+)\s+—\s+(.+?)\s+\(score\s+[\d.]+\)$/;

interface SourceRef {
  path: string;
  location: string;
}

function extractSources(content: string): SourceRef[] {
  const out: SourceRef[] = [];
  for (const line of content.split("\n")) {
    const m = SOURCE_LINE_RE.exec(line);
    if (m) out.push({ path: m[1], location: m[2] });
  }
  return out;
}

function dedupeSources(sources: SourceRef[]): SourceRef[] {
  const seen = new Set<string>();
  const out: SourceRef[] = [];
  for (const s of sources) {
    if (seen.has(s.path)) continue;
    seen.add(s.path);
    out.push(s);
  }
  return out;
}

interface Props {
  isActive: boolean;
}

export function KnowledgeBaseView(props: Props) {
  return (
    <ArtifactPanelProvider>
      <KnowledgeBaseViewInner {...props} />
    </ArtifactPanelProvider>
  );
}

function KnowledgeBaseViewInner({ isActive }: Props) {
  const { t } = useLocale();
  const navigate = useNavigate();
  const { notebooks, loading, syncingIds, syncProgressById, create, remove, sync } = useNotebooks();
  const pythonEnv = usePythonEnvGate();
  const [activeNotebookId, setActiveNotebookId] = useState<string | null>(loadSavedNotebookId);
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [input, setInput] = useState("");
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [submitShortcut, setSubmitShortcut] = useState<SubmitShortcut>("enter");
  const [sourceOpenError, setSourceOpenError] = useState<string | null>(null);
  const submitShortcutRef = useRef<SubmitShortcut>("enter");
  submitShortcutRef.current = submitShortcut;
  const messagesContainerRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const activeNotebook = notebooks.find((nb) => nb.id === activeNotebookId) ?? null;
  const {
    messages, isStreaming, error, isFallbackMode, tokenCount, tokenLimit,
    sessions, activeChatSessionId, send, clear, loadSession, deleteSession,
  } = useKnowledgeBaseChat(activeNotebookId);

  const containerRef = useRef<HTMLDivElement>(null);
  const [historyWidth, setHistoryWidth] = useState(loadSavedHistoryWidth);
  const [isResizingHistory, setIsResizingHistory] = useState(false);
  const [historyCollapsed, setHistoryCollapsed] = useState(loadSavedHistoryCollapsed);

  useEffect(() => { saveHistoryCollapsed(historyCollapsed); }, [historyCollapsed]);

  useEffect(() => {
    if (!isResizingHistory) {
      document.body.style.userSelect = "";
      return;
    }
    document.body.style.userSelect = "none";
    const onMouseMove = (e: MouseEvent) => {
      if (!containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      const newWidth = rect.right - e.clientX;
      setHistoryWidth(Math.max(220, Math.min(newWidth, 480)));
    };
    const onMouseUp = () => {
      setIsResizingHistory(false);
      setHistoryWidth((w) => {
        saveHistoryWidth(w);
        return w;
      });
    };
    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", onMouseUp);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", onMouseUp);
      document.body.style.userSelect = "";
    };
  }, [isResizingHistory]);

  // 首次載入完成後，若儲存的筆記本 id 已不存在（例如被刪除），改選第一個。
  useEffect(() => {
    if (loading) return;
    if (activeNotebookId && notebooks.some((nb) => nb.id === activeNotebookId)) return;
    setActiveNotebookId(notebooks[0]?.id ?? null);
  }, [loading, notebooks, activeNotebookId]);

  useEffect(() => { saveNotebookId(activeNotebookId); }, [activeNotebookId]);

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
      getConfig().then((cfg) => setSubmitShortcut(cfg.submit_shortcut ?? "enter")).catch(() => {});
    }
  }, [isActive]);

  useEffect(() => {
    // Scroll the message list itself rather than calling scrollIntoView on a
    // sentinel inside it. .ca-messages sits several position:absolute/flex
    // layers below the app shell's own overflow:hidden wrapper (App.tsx),
    // none of which are scroll containers — scrollIntoView can walk past
    // .ca-messages and scroll THAT outer wrapper instead (confirmed: with an
    // empty notebook, the hint screen fits within .ca-messages already, so
    // scrollIntoView looks further up for something to adjust). Since that
    // wrapper has overflow:hidden, the user can never scroll it back, so the
    // whole app stays shifted until the window reloads. Same bug class as
    // PythonEnvGate.tsx's log panel; same fix.
    if (isActive) {
      const el = messagesContainerRef.current;
      if (el) el.scrollTop = el.scrollHeight;
    }
  }, [messages, isActive]);

  const handleSend = useCallback(() => {
    if (!input.trim() || isStreaming || !activeNotebookId) return;
    const text = input;
    setInput("");
    void send(text, selectedProviderId || undefined);
  }, [input, isStreaming, activeNotebookId, selectedProviderId, send]);

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

  const handleCreateNotebook = useCallback(async (
    name: string, folderPath: string, embedProviderId?: string, embedModel?: string,
  ) => {
    const nb = await create(name, folderPath, embedProviderId, embedModel);
    setActiveNotebookId(nb.id);
  }, [create]);

  const handleDeleteNotebook = useCallback(async (id: string) => {
    await remove(id);
    if (activeNotebookId === id) setActiveNotebookId(null);
  }, [remove, activeNotebookId]);

  const handleSync = useCallback(async (id: string) => {
    // The import walks the notebook's folder via document_convert, which
    // routes anydoc-covered formats to anydoc but still needs the doc_core
    // profile for anything MarkItDown handles. Unlike DocConverterView, this
    // gate runs unconditionally: a notebook can mix formats, sync failures
    // aren't surfaced per-document in this UI today, and skipping the gate
    // only for notebooks with zero MarkItDown-only files would need a new
    // backend check this plan deliberately doesn't add. See the "Known scope
    // decision" note in docs/superpowers/plans/2026-08-18-anydoc-doc-converter.md.
    const ready = await pythonEnv.ensureProfile("doc_core");
    if (!ready) return;
    await sync(id);
  }, [pythonEnv.ensureProfile, sync]);

  const handleOpenSource = useCallback((path: string) => {
    if (!activeNotebookId) return;
    setSourceOpenError(null);
    kbOpenDocument(activeNotebookId, path).catch((e) => {
      setSourceOpenError(String(e));
    });
  }, [activeNotebookId]);

  return (
    <div className="kb-view" ref={containerRef}>
      <NotebookSidebar
        notebooks={notebooks}
        activeId={activeNotebookId}
        syncingIds={syncingIds}
        syncProgressById={syncProgressById}
        onSelect={setActiveNotebookId}
        onSync={handleSync}
        onDelete={handleDeleteNotebook}
        onAddClick={() => setShowCreateDialog(true)}
      />

      <div className="kb-main">
        <PythonEnvGate
          state={pythonEnv.state}
          lines={pythonEnv.lines}
          error={pythonEnv.error}
          onInstall={() => pythonEnv.ensureProfile("doc_core")}
          onRecheck={() => pythonEnv.ensureProfile("doc_core")}
          onPickInterpreter={() => navigate("/settings", { state: { tab: "general" } })}
          onDismiss={pythonEnv.dismiss}
        />
        {!activeNotebook ? (
          <div className="ca-empty">
            <div className="ca-empty__icon">📚</div>
            <div className="ca-empty__title">{t.kb_empty_title}</div>
            <div className="ca-empty__desc">{t.kb_empty_desc}</div>
            <button className="aiterm-btn aiterm-btn--primary" onClick={() => setShowCreateDialog(true)}>
              {t.kb_create_notebook}
            </button>
          </div>
        ) : (
          <ArtifactSplit>
            {activeNotebook.last_synced_at === null && !syncingIds.has(activeNotebook.id) && (
              <div className="kb-unsynced-banner">
                <span>{t.kb_unsynced_prompt(activeNotebook.name)}</span>
                <button
                  className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                  onClick={() => void handleSync(activeNotebook.id)}
                >
                  {t.kb_sync_button}
                </button>
              </div>
            )}
            <div className="ca-messages" ref={messagesContainerRef}>
              {messages.length === 0 && (
                <div className="ca-hint-center">
                  <div className="ca-hint-title">{t.kb_hint_title}</div>
                  <div className="ca-hint-desc">{t.kb_hint_desc(activeNotebook.name)}</div>
                  <div className="ca-hint-examples">
                    {t.kb_hint_examples.map((ex) => (
                      <button key={ex} className="ca-hint-chip" onClick={() => setInput(ex)}>
                        {ex}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {messages.map((msg, i) => {
                const isDone = msg.role === "assistant" && !msg.streaming;
                const sources = isDone
                  ? dedupeSources([
                      ...(msg.toolCalls ?? [])
                        .filter((tc) => tc.tool === "search_documents" && tc.result && !tc.result.content.startsWith("Error:"))
                        .flatMap((tc) => extractSources(tc.result!.content)),
                      ...(msg.toolCalls ?? [])
                        .filter((tc) => tc.tool === "read_document" && tc.result && !tc.result.content.startsWith("Error:"))
                        .map((tc) => ({ path: String(tc.args.path ?? ""), location: "" })),
                    ])
                  : [];

                return (
                  <div key={i} className={`ca-msg ca-msg--${msg.role}`}>
                    {msg.role === "assistant" && (msg.toolCalls ?? []).map((tc) => (
                      <ToolCallCard key={tc.callId} toolCall={tc} />
                    ))}
                    {msg.role === "assistant" && (msg.checkpoints ?? []).map((n) => (
                      <div key={`checkpoint-${n}`} className="kb-checkpoint-badge">
                        {t.kb_checkpoint_notice(n)}
                      </div>
                    ))}
                    {(msg.content || msg.streaming) && (
                      <div className="ca-msg__bubble">
                        {msg.role === "assistant" && msg.streaming && !msg.content ? (
                          <span className="ca-thinking-indicator"><span /><span /><span /></span>
                        ) : (
                          <>
                            {msg.role === "assistant" ? <MarkdownText text={msg.content} /> : msg.content}
                            {msg.streaming && <span className="ca-streaming-cursor" />}
                          </>
                        )}
                      </div>
                    )}
                    {sources.length > 0 && (
                      <div className="kb-sources">
                        {sources.map((s) => (
                          <button
                            key={s.path}
                            className="kb-sources__chip"
                            title={s.location ? `${s.path} — ${s.location}` : s.path}
                            onClick={() => handleOpenSource(s.path)}
                          >
                            {s.path.split("/").pop()}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                );
              })}
              {error && <div className="ca-error">{error}</div>}
              {sourceOpenError && <div className="ca-error">{sourceOpenError}</div>}
            </div>

            {isFallbackMode && <div className="ca-fallback-banner">{t.ca_fallback_banner}</div>}

            <div className="ca-toolbar">
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
                <button
                  className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                  onClick={() => { if (!isStreaming) clear(); }}
                >
                  {t.ca_clear}
                </button>
              )}
            </div>

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
                  placeholder={t.kb_input_placeholder(shortcutLabel)}
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
        )}
      </div>

      {activeNotebook && (
        <>
          {!historyCollapsed && (
            <div
              className="kb-chat-history-resizer"
              onMouseDown={(e) => { e.preventDefault(); setIsResizingHistory(true); }}
            />
          )}
          <ChatHistorySidebar
            width={historyCollapsed ? HISTORY_COLLAPSED_WIDTH : historyWidth}
            notebookName={activeNotebook.name}
            sessions={sessions}
            activeSessionId={activeChatSessionId}
            isStreaming={isStreaming}
            collapsed={historyCollapsed}
            onNew={() => { if (!isStreaming) clear(); }}
            onSelect={loadSession}
            onDelete={deleteSession}
            onToggleCollapse={() => setHistoryCollapsed((c) => !c)}
          />
        </>
      )}

      {showCreateDialog && (
        <NotebookCreateDialog
          onCreate={handleCreateNotebook}
          onClose={() => setShowCreateDialog(false)}
        />
      )}
    </div>
  );
}
