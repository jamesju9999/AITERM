import { useState, useEffect, useRef, useMemo } from "react";
import { dbListTables, dbExecuteQuery, type TableInfo, type QueryResult } from "../../ipc/db";
import { aiChat, formatAiError, type AiError } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { getConfig, type SubmitShortcut } from "../../ipc/config";
import { extractResponseText, unescapeNewlines, MarkdownText } from "../../lib/markdown";
import { parseSchemaDoc, buildSchemaSection } from "../../lib/schemaDoc";
import { useLocale } from "../../contexts/LocaleContext";
import { ModelPickerButton } from "../ModelPickerButton";
import { ArtifactPanelProvider } from "../../contexts/ArtifactPanelContext";
import { ArtifactSplit } from "../ArtifactPanel/ArtifactSplit";

interface Props {
  connectionId: string;
  schema: string;
  sendRemoteResponse?: (text: string) => void;
}

interface AgentStep {
  sql: string;
  result?: QueryResult;
  executing: boolean;
  collapsed: boolean;
}

interface Message {
  role: "user" | "assistant";
  text: string;
  // Agentic: intermediate SQL steps + final answer
  steps?: AgentStep[];
  agentRunning?: boolean;
  agentStepLabel?: string;
}

interface SavedSession {
  id: string;
  title: string;
  messages: Message[];
  savedAt: number;
}

function chatStorageKey(connectionId: string) {
  return `aiterm-db-chat-history-${connectionId}`;
}

function loadSessions(connectionId: string): SavedSession[] {
  try {
    const raw = localStorage.getItem(chatStorageKey(connectionId));
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveSessions(connectionId: string, sessions: SavedSession[]) {
  localStorage.setItem(chatStorageKey(connectionId), JSON.stringify(sessions.slice(-50)));
}

function schemaDocKey(connectionId: string) {
  return `aiterm-db-schema-doc-${connectionId}`;
}

function loadSchemaDoc(connectionId: string): string {
  return localStorage.getItem(schemaDocKey(connectionId)) ?? "";
}

function saveSchemaDoc(connectionId: string, content: string) {
  localStorage.setItem(schemaDocKey(connectionId), content);
}

function extractSql(text: string): string | null {
  const sqlBlock = text.match(/```sql\s*([\s\S]*?)```/i);
  if (sqlBlock) return sqlBlock[1].trim();

  const jsonBlock = text.match(/```(?:json)?\s*(\{[\s\S]*?\})\s*```/i);
  if (jsonBlock) {
    try {
      const parsed = JSON.parse(jsonBlock[1]);
      if (typeof parsed.sql === "string" && parsed.sql.trim()) return parsed.sql.trim();
    } catch { /* not valid JSON */ }
  }

  const bareJson = text.match(/\{[\s\S]*?"sql"\s*:\s*"([\s\S]*?)"[\s\S]*?\}/);
  if (bareJson) {
    try {
      const parsed = JSON.parse(bareJson[0]);
      if (typeof parsed.sql === "string" && parsed.sql.trim()) return parsed.sql.trim();
    } catch { /* not valid JSON */ }
  }

  // Fallback: <cmd> tag — model may wrap a shell DB2/sql CLI call
  const cmdTag = text.match(/<cmd>([\s\S]*?)<\/cmd>/i);
  if (cmdTag) {
    const inner = cmdTag[1].trim();
    // DB2 CLI: sql 'SELECT ...' or db2 "SELECT ..."
    const cliMatch = inner.match(/(?:^|\s)(?:sql|db2)\s+['"](.+?)['"](?:\s*[;|]|$)/is);
    if (cliMatch) return cliMatch[1].replace(/''/g, "'").trim();
    // PowerShell wrapper: -Command "sql '...'"
    const pwshMatch = inner.match(/-Command\s+["'](?:sql|db2)\s+['"](.+?)['"](?:['"]\s*\|.*)?$/is);
    if (pwshMatch) return pwshMatch[1].replace(/''/g, "'").trim();
    // The <cmd> content itself looks like SQL
    if (/^\s*(SELECT|INSERT|UPDATE|DELETE|WITH|CREATE|DROP|ALTER)\s/i.test(inner)) return inner;
    // AITerm cross-db syntax: sql [connectionName] SELECT ...
    const crossDbMatch = inner.match(/^\s*(?:sql|db2)\s+\[.+?\]\s+([\s\S]+)$/i);
    if (crossDbMatch) return crossDbMatch[1].trim();
  }

  return null;
}

function normalizeAiError(err: unknown): AiError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as AiError;
  }
  if (err instanceof Error) {
    try {
      const parsed = JSON.parse(err.message);
      if (parsed && typeof parsed === "object" && "kind" in parsed) {
        return parsed as AiError;
      }
    } catch {
      // fall through
    }
    return { kind: "network", message: err.message };
  }
  return { kind: "network", message: String(err) };
}

function errorText(err: unknown): string {
  return formatAiError(normalizeAiError(err));
}

export function DatabaseAiChat(props: Props) {
  return (
    <ArtifactPanelProvider>
      <DatabaseAiChatInner {...props} />
    </ArtifactPanelProvider>
  );
}

function DatabaseAiChatInner({ connectionId, schema, sendRemoteResponse }: Props) {
  const { t, locale } = useLocale();
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [tables, setTables] = useState<TableInfo[]>([]);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState<string>("");
  const [historyOpen, setHistoryOpen] = useState(false);
  const [submitShortcut, setSubmitShortcut] = useState<SubmitShortcut>("enter");
  const [sessions, setSessions] = useState<SavedSession[]>([]);
  const maxStepsRef = useRef<number>(5);
  const [schemaDoc, setSchemaDoc] = useState<string>(() => loadSchemaDoc(connectionId));
  const schemaDocRef = useRef(schemaDoc);
  useEffect(() => { schemaDocRef.current = schemaDoc; }, [schemaDoc]);
  const schemaDocMap = useMemo(() => parseSchemaDoc(schemaDoc), [schemaDoc]);
  const schemaFileInputRef = useRef<HTMLInputElement>(null);
  const messagesContainerRef = useRef<HTMLDivElement>(null);
  const stoppedRef = useRef(false);
  const currentSessionIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (schema) {
      dbListTables(connectionId, schema).then(setTables).catch(console.error);
    }
  }, [connectionId, schema]);

  useEffect(() => {
    const onRemoteMsg = (e: Event) => {
      const text = (e as CustomEvent).detail.text;
      if (text) {
        setInput(text);
        setTimeout(() => {
          const btn = document.getElementById("db-ai-send-btn");
          if (btn) btn.click();
        }, 50);
      }
    };
    window.addEventListener("aiterm:db-remote-msg", onRemoteMsg);
    return () => window.removeEventListener("aiterm:db-remote-msg", onRemoteMsg);
  }, []);

  useEffect(() => {
    getConfig().then((cfg) => {
      maxStepsRef.current = cfg.max_agent_steps === 0 ? 9999 : (cfg.max_agent_steps ?? 5);
      setSubmitShortcut(cfg.submit_shortcut ?? "enter");
    }).catch(() => {});
  }, []);

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      const def = list.find((p) => p.is_default);
      if (def) setSelectedProviderId(def.id);
    }).catch(console.error);
  }, []);

  useEffect(() => {
    setSessions(loadSessions(connectionId));
  }, [connectionId]);

  useEffect(() => {
    setSchemaDoc(loadSchemaDoc(connectionId));
  }, [connectionId]);

  useEffect(() => {
    const el = messagesContainerRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages]);

  const formatResultForAi = (result: QueryResult): string => {
    if (result.error) return `${t.db_ai_result_err}${result.error}`;
    if (result.affected_rows !== null) return t.db_ai_result_affected(result.affected_rows);
    if (result.columns.length === 0) return t.db_ai_result_empty;
    const header = result.columns.join(" | ");
    const rows = result.rows.slice(0, 50).map((r) => r.map((c) => (c === null ? "NULL" : String(c))).join(" | "));
    const suffix = result.rows.length > 50 ? t.db_ai_result_truncated(result.rows.length) : "";
    return `${header}\n${rows.join("\n")}${suffix}`;
  };

  const buildSystemPrompt = (userQuestion = "") => {
    const tableList = tables.map((t) => t.name).join(", ");
    const maxSteps = maxStepsRef.current;
    const tableNames = tables.map((t) => t.name);
    const schemaSection = buildSchemaSection(schemaDocMap, tableNames, userQuestion, 6000);
    const maxStepsStr = maxSteps >= 9999 ? (locale === "zh-TW" ? "不限次數" : "unlimited") : String(maxSteps);

    return t.db_ai_system_prompt(schema, tableList, maxStepsStr) +
      (schemaSection ? "\n" + schemaSection + "\n" : "");
  };

  const updateLastMsg = (updater: (m: Message) => Message) => {
    setMessages((prev) => {
      const copy = [...prev];
      copy[copy.length - 1] = updater(copy[copy.length - 1]);
      return copy;
    });
  };

  const persistSession = (msgs: Message[], sessionId: string, title: string) => {
    const updated: SavedSession = { id: sessionId, title, messages: msgs, savedAt: Date.now() };
    setSessions((prev) => {
      const others = prev.filter((s) => s.id !== sessionId);
      const next = [...others, updated];
      saveSessions(connectionId, next);
      return next;
    });
  };

  const send = async () => {
    if (!input.trim() || sending) return;
    const userMsg = input.trim();
    setInput("");
    setSending(true);
    stoppedRef.current = false;

    if (!currentSessionIdRef.current) {
      currentSessionIdRef.current = crypto.randomUUID();
    }
    const sessionId = currentSessionIdRef.current;
    const sessionTitle = userMsg.length > 40 ? userMsg.slice(0, 40) + "…" : userMsg;

    const historyForAi = messages
      .filter((m) => !m.agentRunning)
      .map((m) => ({ role: m.role, content: m.text }));

    const userMessage: Message = { role: "user", text: userMsg };
    const assistantMessage: Message = {
      role: "assistant",
      text: "",
      steps: [],
      agentRunning: true,
      agentStepLabel: t.db_ai_thinking_n(1, maxStepsRef.current >= 9999 ? "" : "/" + maxStepsRef.current),
    };

    const nextMessages = [...messages, userMessage, assistantMessage];
    setMessages(nextMessages);
    persistSession(nextMessages, sessionId, sessionTitle);

    const loopHistory: { role: "user" | "assistant"; content: string }[] = [
      ...historyForAi,
      { role: "user", content: userMsg },
    ];

    try {
      let stepCount = 0;
      const maxSteps = maxStepsRef.current;
      let finalMessages = nextMessages;

      const updateAndPersist = (updater: (m: Message) => Message) => {
        setMessages((prev) => {
          const copy = [...prev];
          copy[copy.length - 1] = updater(copy[copy.length - 1]);
          finalMessages = copy;
          persistSession(copy, sessionId, sessionTitle);
          return copy;
        });
      };

      while (stepCount < maxSteps && !stoppedRef.current) {
        const stepLabel = t.db_ai_thinking_n(stepCount + 1, maxSteps >= 9999 ? "" : "/" + maxSteps);
        updateLastMsg((m) => ({ ...m, agentStepLabel: stepLabel }));

        const lastUserContent = loopHistory[loopHistory.length - 1].content;
        const aiResult = await aiChat(
          [{ role: "system" as const, content: buildSystemPrompt(userMsg) }, ...loopHistory.slice(0, -1), { role: "user" as const, content: lastUserContent }],
          `db-${connectionId}`,
          selectedProviderId || undefined,
          false,
          locale,
          true,
        );
        const reply = aiResult.content ?? "";

        if (stoppedRef.current) break;

        const sql = extractSql(reply);

        if (!sql) {
          updateAndPersist((m) => ({
            ...m,
            text: reply,
            agentRunning: false,
            agentStepLabel: undefined,
          }));
          if (sendRemoteResponse) sendRemoteResponse(reply);
          break;
        }

        const stepIndex = stepCount;
        updateLastMsg((m) => ({
          ...m,
          steps: [...(m.steps ?? []), { sql, executing: true, collapsed: false }],
        }));

        let result: QueryResult;
        try {
          result = await dbExecuteQuery(connectionId, sql, schema || undefined);
        } catch (e: unknown) {
          result = { columns: [], rows: [], affected_rows: null, execution_time_ms: 0, error: errorText(e) };
        }

        if (stoppedRef.current) break;

        updateLastMsg((m) => ({
          ...m,
          steps: (m.steps ?? []).map((s, i) =>
            i === stepIndex ? { ...s, result, executing: false, collapsed: false } : s
          ),
        }));

        loopHistory.push({ role: "assistant", content: reply });
        loopHistory.push({
          role: "user",
          content: t.db_ai_loop_result(formatResultForAi(result)),
        });

        stepCount++;

        if (stepCount >= maxSteps) {
          updateLastMsg((m) => ({ ...m, agentStepLabel: t.db_ai_summarizing_label }));
          const summaryResult = await aiChat(
            [{ role: "system" as const, content: buildSystemPrompt(userMsg) }, ...loopHistory, { role: "user" as const, content: t.db_ai_summarizing_prompt }],
            `db-${connectionId}`,
            selectedProviderId || undefined,
            false,
            locale,
            true,
          );
          const summary = summaryResult.content ?? "";
          updateAndPersist((m) => ({
            ...m,
            text: summary,
            agentRunning: false,
            agentStepLabel: undefined,
          }));
          if (sendRemoteResponse) sendRemoteResponse(summary);
        }
      }

      if (stoppedRef.current) {
        updateAndPersist((m) => ({
          ...m,
          text: m.text || t.db_ai_stopped,
          agentRunning: false,
          agentStepLabel: undefined,
        }));
        if (sendRemoteResponse && !finalMessages[finalMessages.length - 1].text) {
          sendRemoteResponse(t.db_ai_stopped);
        }
      }

      void finalMessages;
    } catch (e: unknown) {
      setMessages((prev) => {
        const copy = [...prev];
        copy[copy.length - 1] = {
          ...copy[copy.length - 1],
          text: t.db_ai_error(errorText(e)),
          agentRunning: false,
          agentStepLabel: undefined,
        };
        persistSession(copy, sessionId, sessionTitle);
        return copy;
      });
    } finally {
      setSending(false);
    }
  };

  const stop = () => { stoppedRef.current = true; };

  const loadSession = (session: SavedSession) => {
    setMessages(session.messages);
    currentSessionIdRef.current = session.id;
    setHistoryOpen(false);
  };

  const deleteSession = (id: string, e: React.MouseEvent) => {
    e.stopPropagation();
    setSessions((prev) => {
      const next = prev.filter((s) => s.id !== id);
      saveSessions(connectionId, next);
      return next;
    });
    if (currentSessionIdRef.current === id) {
      setMessages([]);
      currentSessionIdRef.current = null;
    }
  };

  const newChat = () => {
    setMessages([]);
    currentSessionIdRef.current = null;
    setHistoryOpen(false);
  };

  const handleSchemaUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    file.text().then((content) => {
      saveSchemaDoc(connectionId, content);
      setSchemaDoc(content);
    }).catch(console.error);
    e.target.value = "";
  };

  const removeSchemaDoc = () => {
    localStorage.removeItem(schemaDocKey(connectionId));
    setSchemaDoc("");
  };

  const formatDate = (ts: number): string => {
    const d = new Date(ts);
    return d.toLocaleDateString(locale === "zh-TW" ? "zh-TW" : "en-US", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
  };

  return (
    <div style={{ display: "flex", height: "100%", flex: 1, minWidth: 0, position: "relative" }}>
      {/* History side panel */}
      {historyOpen && (
        <div style={{
          width: 240, borderRight: "1px solid #1e1e1e", background: "#0e0e0e",
          display: "flex", flexDirection: "column", flexShrink: 0,
        }}>
          <div style={{ padding: "8px 12px", borderBottom: "1px solid #1e1e1e", display: "flex", alignItems: "center", justifyContent: "space-between" }}>
            <span style={{ fontSize: 11, color: "#555", textTransform: "uppercase", letterSpacing: "0.05em" }}>{t.cdb_ai_history_title}</span>
            <button
              onClick={newChat}
              className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            >
              {t.cdb_ai_history_new_btn}
            </button>
          </div>
          <div style={{ flex: 1, overflowY: "auto" }}>
            {sessions.length === 0 && (
              <div style={{ color: "#444", fontSize: 12, padding: "16px 12px" }}>{t.cdb_ai_history_empty}</div>
            )}
            {[...sessions].reverse().map((s) => (
              <div
                key={s.id}
                onClick={() => loadSession(s)}
                style={{
                  padding: "8px 12px", cursor: "pointer", borderBottom: "1px solid #161616",
                  display: "flex", alignItems: "flex-start", gap: 6,
                  background: currentSessionIdRef.current === s.id ? "#1a2030" : "transparent",
                }}
                onMouseEnter={(e) => { if (currentSessionIdRef.current !== s.id) (e.currentTarget as HTMLDivElement).style.background = "#161616"; }}
                onMouseLeave={(e) => { (e.currentTarget as HTMLDivElement).style.background = currentSessionIdRef.current === s.id ? "#1a2030" : "transparent"; }}
              >
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 12, color: "#ccc", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{s.title}</div>
                  <div style={{ fontSize: 10, color: "#444", marginTop: 2 }}>{formatDate(s.savedAt)}</div>
                </div>
                <button
                  onClick={(e) => deleteSession(s.id, e)}
                  title={t.cdb_ai_delete_tooltip}
                  className="db-ai-session-delete-btn aiterm-btn aiterm-btn--ghost"
                  style={{ color: "#444", fontSize: 14, padding: "2px 5px" }}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Main chat area */}
      <ArtifactSplit>
      <div style={{ display: "flex", flexDirection: "column", flex: 1, minWidth: 0 }}>
        <div ref={messagesContainerRef} style={{ flex: 1, overflowY: "auto", padding: "12px 16px", display: "flex", flexDirection: "column", gap: 12 }}>
          {messages.length === 0 && (
            <div style={{ color: "#555", fontSize: 13, padding: "20px 0" }}>
              {t.db_ai_welcome_hint}
            </div>
          )}
          {messages.map((msg, i) => (
            <MessageBubble
              key={i}
              msg={msg}
              onToggleStep={(stepIdx) => {
                setMessages((prev) =>
                  prev.map((m, mi) =>
                    mi !== i ? m : {
                      ...m,
                      steps: (m.steps ?? []).map((s, si) =>
                        si === stepIdx ? { ...s, collapsed: !s.collapsed } : s
                      ),
                    }
                  )
                );
              }}
            />
          ))}
        </div>

        {/* Toolbar: provider selector + history toggle */}
        <div style={{ borderTop: "1px solid #1e1e1e", padding: "6px 12px", display: "flex", alignItems: "center", gap: 8, background: "#111" }}>
          <ModelPickerButton
            providers={providers}
            selectedId={selectedProviderId}
            onChange={setSelectedProviderId}
          />
          <input
            ref={schemaFileInputRef}
            type="file"
            accept=".md"
            style={{ display: "none" }}
            onChange={handleSchemaUpload}
          />
          <button
            onClick={() => schemaFileInputRef.current?.click()}
            title={schemaDoc ? t.db_ai_schema_tooltip_change : t.db_ai_schema_tooltip_upload}
            style={{
              background: schemaDoc ? "#1a2a1e" : "transparent",
              border: "1px solid " + (schemaDoc ? "#34d399" : "#2a2a2a"),
              color: schemaDoc ? "#34d399" : "#555",
              borderRadius: 4, padding: "2px 8px", fontSize: 11, cursor: "pointer",
            }}
          >
            📄 Schema{schemaDoc ? " ✓" : ""}
          </button>
          {schemaDoc && (
            <button
              onClick={removeSchemaDoc}
              title={t.db_ai_schema_tooltip_remove}
              className="db-ai-schema-remove-btn aiterm-btn aiterm-btn--ghost"
              style={{ color: "#555", fontSize: 12, padding: "2px 4px" }}
            >
              ×
            </button>
          )}
          <div style={{ flex: 1 }} />
          <button
            onClick={() => setHistoryOpen((o) => !o)}
            title={t.cdb_ai_history_title}
            style={{
              background: historyOpen ? "#1a2030" : "transparent",
              border: "1px solid " + (historyOpen ? "#3b5bdb" : "#2a2a2a"),
              color: historyOpen ? "#74b9ff" : "#555",
              borderRadius: 4, padding: "2px 8px", fontSize: 11, cursor: "pointer",
            }}
          >
            {t.db_ai_btn_history}
          </button>
          {messages.length > 0 && (
            <button
              onClick={newChat}
              title={t.cdb_ai_history_new_btn}
              className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            >
              {t.cdb_ai_history_new_btn}
            </button>
          )}
        </div>

        {/* Input bar */}
        <div style={{ borderTop: "1px solid var(--border-color)", padding: "10px 16px 12px", display: "flex", gap: 10, alignItems: "center", background: "rgba(10, 11, 20, 0.4)" }}>
          <div className="aiterm-input-pill-container" style={{ display: "flex", alignItems: "center", background: "rgba(255, 255, 255, 0.03)", border: "1px solid rgba(255, 255, 255, 0.08)", borderRadius: 20, padding: "4px 8px", flex: 1, gap: 6 }}>
            <textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  const ok = (submitShortcut === "enter" && !e.shiftKey && !e.ctrlKey && !e.metaKey) ||
                             (submitShortcut === "shift-enter" && e.shiftKey && !e.ctrlKey) ||
                             (submitShortcut === "ctrl-enter" && (e.ctrlKey || e.metaKey) && !e.shiftKey);
                  if (ok) { e.preventDefault(); send(); }
                }
              }}
              placeholder={t.db_ai_input_placeholder}
              rows={1}
              style={{
                flex: 1, background: "transparent", border: "none", color: "var(--text-primary)",
                padding: "4px 6px", fontSize: 13, resize: "none", outline: "none",
                fontFamily: "inherit", height: 24, lineHeight: "24px", overflowY: "hidden"
              }}
            />
            {sending ? (
              <button
                onClick={stop}
                className="aiterm-btn aiterm-btn--danger-solid aiterm-btn--icon"
                title={t.db_ai_btn_stop}
              >
                ■
              </button>
            ) : (
              <button
                id="db-ai-send-btn"
                onClick={send}
                disabled={!input.trim()}
                className="aiterm-btn aiterm-btn--primary aiterm-btn--icon"
                title={t.db_ai_btn_send}
              >
                ▲
              </button>
            )}
          </div>
        </div>
      </div>
      </ArtifactSplit>
    </div>
  );
}

function MessageBubble({ msg, onToggleStep }: { msg: Message; onToggleStep: (i: number) => void }) {
  const { t } = useLocale();
  const [copied, setCopied] = useState(false);
  const handleCopy = () => {
    const text = unescapeNewlines(extractResponseText(msg.text ?? ""));
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };
  if (msg.role === "user") {
    return (
      <div style={{ display: "flex", justifyContent: "flex-end" }}>
        <div style={{
          background: "#1a2a3e", border: "1px solid #60a5fa33",
          borderRadius: 8, padding: "8px 12px", maxWidth: "80%", fontSize: 13, color: "#e6e6e6",
        }}>
          {msg.text}
        </div>
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6, alignItems: "flex-start", maxWidth: "95%" }}>
      {(msg.steps ?? []).map((step, i) => (
        <div key={i} style={{ width: "100%", background: "#141414", border: "1px solid #252525", borderRadius: 6, overflow: "hidden" }}>
          <button
            onClick={() => onToggleStep(i)}
            style={{
              width: "100%", background: "transparent", border: "none", display: "flex",
              alignItems: "center", gap: 6, padding: "6px 10px", cursor: "pointer", color: "#666",
              fontSize: 11, textAlign: "left",
            }}
          >
            <span style={{ color: step.result?.error ? "#f87171" : "#34d39988" }}>
              {step.executing ? "⟳" : step.result?.error ? "✗" : "✓"}
            </span>
            <span style={{ fontFamily: "monospace", flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {t.db_ai_step_n(i + 1)}{step.sql}
            </span>
            <span>{step.collapsed ? "▶" : "▼"}</span>
          </button>
          {!step.collapsed && (
            <div style={{ borderTop: "1px solid #1e1e1e", padding: "8px 10px" }}>
              <div style={{ fontFamily: "monospace", fontSize: 11, color: "#34d399", marginBottom: 6, whiteSpace: "pre-wrap" }}>
                {step.sql}
              </div>
              {step.executing && <div style={{ color: "#888", fontSize: 11 }}>{t.db_sql_running}</div>}
              {step.result && !step.executing && <ResultInline result={step.result} />}
            </div>
          )}
        </div>
      ))}

      {msg.agentRunning && (
        <div style={{ color: "#888", fontSize: 11, padding: "4px 0" }}>
          <span style={{ marginRight: 6 }}>⟳</span>{msg.agentStepLabel}
        </div>
      )}

      {msg.text && !msg.agentRunning && (
        <div style={{
          background: "#1a1a1a", border: "1px solid #2a2a2a",
          borderRadius: 8, padding: "8px 12px", fontSize: 13, color: "#e6e6e6",
          position: "relative",
        }} className="db-ai-answer db-ai-answer--copyable">
          <MarkdownText text={unescapeNewlines(extractResponseText(msg.text)).replace(/<cmd>([\s\S]*?)<\/cmd>/gi, (_m, c) => `\`\`\`\n${c.trim()}\n\`\`\``)} />
          <button
            type="button"
            className={`db-ai-copy-btn aiterm-btn aiterm-btn--secondary aiterm-btn--sm${copied ? " db-ai-copy-btn--copied" : ""}`}
            onClick={handleCopy}
            title={t.db_ai_copy_tooltip}
          >{copied ? "✓" : "⎘"}</button>
        </div>
      )}
      {msg.text && msg.agentRunning && (
        <div style={{
          background: "#1a1a1a", border: "1px solid #2a2a2a",
          borderRadius: 8, padding: "8px 12px", fontSize: 13, color: "#e6e6e6",
        }} className="db-ai-answer">
          <MarkdownText text={unescapeNewlines(extractResponseText(msg.text)).replace(/<cmd>([\s\S]*?)<\/cmd>/gi, (_m, c) => `\`\`\`\n${c.trim()}\n\`\`\``)} />
        </div>
      )}
    </div>
  );
}

function ResultInline({ result }: { result: QueryResult }) {
  const { t } = useLocale();
  if (result.error) {
    return <div style={{ color: "#f87171", fontSize: 11 }}>✗ {result.error}</div>;
  }
  if (result.affected_rows !== null) {
    return <div style={{ color: "#34d399", fontSize: 11 }}>✓ {t.db_ai_result_affected(result.affected_rows)} ({result.execution_time_ms}ms)</div>;
  }
  if (result.columns.length === 0) return null;
  return (
    <div style={{ overflowX: "auto" }}>
      <div style={{ color: "#555", fontSize: 10, marginBottom: 4 }}>{result.rows.length} {t.db_rows} · {result.execution_time_ms}ms</div>
      <table style={{ borderCollapse: "collapse", fontSize: 11, fontFamily: "monospace" }}>
        <thead>
          <tr>{result.columns.map((c) => (
            <th key={c} style={{ padding: "2px 8px", color: "#666", borderBottom: "1px solid #222", textAlign: "left", fontWeight: "normal" }}>{c}</th>
          ))}</tr>
        </thead>
        <tbody>
          {result.rows.slice(0, 20).map((row, i) => (
            <tr key={i} style={{ background: i % 2 === 0 ? "transparent" : "#0f0f0f" }}>
              {row.map((cell, j) => (
                <td key={j} style={{ padding: "2px 8px", color: cell === null ? "#444" : "#bbb", borderBottom: "1px solid #111" }}>
                  {cell === null ? "NULL" : String(cell)}
                </td>
              ))}
            </tr>
          ))}
          {result.rows.length > 20 && (
            <tr><td colSpan={result.columns.length} style={{ padding: "3px 8px", color: "#444", fontSize: 10 }}>{t.db_ai_more_rows(result.rows.length - 20)}</td></tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
