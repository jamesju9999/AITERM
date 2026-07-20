import { useState, useEffect, useRef } from "react";
import { dbExecuteQuery, type QueryResult } from "../../ipc/db";
import { aiChat, formatAiError, type AiError } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { getConfig, type SubmitShortcut } from "../../ipc/config";
import { extractResponseText, unescapeNewlines, MarkdownText } from "../../lib/markdown";
import { useLocale } from "../../contexts/LocaleContext";
import { ModelPickerButton } from "../ModelPickerButton";
import type { ConnectedDb } from "./index";

interface Props {
  databases: ConnectedDb[];
  sendRemoteResponse?: (text: string) => void;
}

interface SavedSession {
  id: string;
  title: string;
  messages: Message[];
  savedAt: number;
}

const CROSSDB_SESSIONS_KEY = "aiterm-crossdb-chat-sessions";

function loadSessions(): SavedSession[] {
  try {
    const raw = localStorage.getItem(CROSSDB_SESSIONS_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveSessions(sessions: SavedSession[]): void {
  try {
    localStorage.setItem(CROSSDB_SESSIONS_KEY, JSON.stringify(sessions));
  } catch { /* ignore */ }
}

interface AgentStep {
  targetDb: string;
  sql: string;
  result?: QueryResult;
  executing: boolean;
  collapsed: boolean;
}

interface Message {
  role: "user" | "assistant";
  text: string;
  steps?: AgentStep[];
  agentRunning?: boolean;
  agentStepLabel?: string;
}

function extractCrossDbSql(text: string): { alias: string | null; sql: string } | null {
  // 1. ```sql [DB-Name]\nSQL\n```  (with square brackets — spec format)
  const match = text.match(/```sql\s+\[(.+?)\]\s*\n([\s\S]*?)```/i);
  if (match) return { alias: match[1].trim(), sql: match[2].trim() };

  // 2. ```sql DB-Name\nSQL\n```  (without square brackets — gemma omits them)
  const nobrack = text.match(/```sql\s+([^\[\n]+?)\s*\n([\s\S]*?)```/i);
  if (nobrack) {
    const sql = nobrack[2].trim();
    if (/^(SELECT|INSERT|UPDATE|DELETE|CREATE|DROP|ALTER|WITH)\b/i.test(sql)) {
      return { alias: nobrack[1].trim(), sql };
    }
  }

  // 3. ```sql\n[DB-Name]\nSQL\n```
  const fallback = text.match(/```sql\s*\n\[(.+?)\]\s*\n([\s\S]*?)```/i);
  if (fallback) return { alias: fallback[1].trim(), sql: fallback[2].trim() };

  // 4. <cmd> sql [DB-Name] SELECT ... </cmd>
  const cmdTag = text.match(/<cmd>([\s\S]*?)<\/cmd>/i);
  if (cmdTag) {
    const inner = cmdTag[1].trim();
    const crossDbMatch = inner.match(/^\s*(?:sql|db2)\s+\[(.+?)\]\s+([\s\S]+)$/i);
    if (crossDbMatch) return { alias: crossDbMatch[1].trim(), sql: crossDbMatch[2].trim() };
  }

  // 5. Plain ```sql\nSELECT...``` without DB name
  const plainSql = text.match(/```(?:sql)?\s*\n([\s\S]*?)```/i);
  if (plainSql) {
    const sql = plainSql[1].trim();
    if (/^(SELECT|INSERT|UPDATE|DELETE|CREATE|DROP|ALTER|WITH)\b/i.test(sql)) {
      return { alias: null, sql };
    }
  }

  // 6. Bare SQL with no code fences — scan lines, stop before any closing fence
  const lines = text.split('\n');
  let preambleCount = 0;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line) continue;
    if (/^(SELECT|INSERT|UPDATE|DELETE|CREATE|DROP|ALTER|WITH)\b/i.test(line)) {
      if (preambleCount <= 3) {
        // Collect SQL lines until a ``` fence or end of text
        const sqlLines: string[] = [];
        for (let j = i; j < lines.length; j++) {
          if (lines[j].trim().startsWith('```')) break;
          sqlLines.push(lines[j]);
        }
        return { alias: null, sql: sqlLines.join('\n').trim() };
      }
      break;
    }
    preambleCount++;
  }

  return null;
}

function normalizeAiError(err: unknown): AiError {
  if (err && typeof err === "object" && "kind" in err) return err as AiError;
  if (err instanceof Error) {
    try {
      const parsed = JSON.parse(err.message);
      if (parsed && typeof parsed === "object" && "kind" in parsed) return parsed as AiError;
    } catch { /* ignore */ }
    return { kind: "network", message: err.message };
  }
  return { kind: "network", message: String(err) };
}

export function CrossDbAiChat({ databases, sendRemoteResponse }: Props) {
  const { t, locale } = useLocale();
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState<string>("");
  const [sessions, setSessions] = useState<SavedSession[]>(loadSessions);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [submitShortcut, setSubmitShortcut] = useState<SubmitShortcut>("enter");
  const maxStepsRef = useRef<number>(5);
  const messagesContainerRef = useRef<HTMLDivElement>(null);
  const stoppedRef = useRef(false);
  const currentSessionIdRef = useRef<string | null>(null);

  useEffect(() => {
    const onRemoteMsg = (e: Event) => {
      const text = (e as CustomEvent).detail.text;
      if (text) {
        setInput(text);
        setTimeout(() => {
          const btn = document.getElementById("crossdb-ai-send-btn");
          if (btn) btn.click();
        }, 50);
      }
    };
    window.addEventListener("aiterm:crossdb-remote-msg", onRemoteMsg);
    return () => window.removeEventListener("aiterm:crossdb-remote-msg", onRemoteMsg);
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
    const el = messagesContainerRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages]);

  // Auto-save sessions when messages settle (skip while agent is still running)
  useEffect(() => {
    if (messages.length === 0) return;
    if (messages.some((m) => m.agentRunning)) return;
    const title = messages.find((m) => m.role === "user")?.text.slice(0, 30) ?? "（空對話）";
    if (!currentSessionIdRef.current) {
      currentSessionIdRef.current = `crossdb-${Date.now()}`;
    }
    const id = currentSessionIdRef.current;
    const updated: SavedSession = { id, title, messages, savedAt: Date.now() };
    const all = loadSessions();
    const idx = all.findIndex((s) => s.id === id);
    const next = idx >= 0 ? all.map((s) => (s.id === id ? updated : s)) : [...all, updated];
    saveSessions(next);
    setSessions(next);
  }, [messages]);

  const formatResultForAi = (result: QueryResult): string => {
    if (result.error) return `${t.cdb_ai_result_err}${result.error}`;
    if (result.affected_rows !== null) return t.cdb_ai_result_affected(result.affected_rows);
    if (result.columns.length === 0) return t.cdb_ai_result_empty;
    const header = result.columns.join(" | ");
    const rows = result.rows.slice(0, 50).map((r) => r.map((c) => (c === null ? "NULL" : String(c))).join(" | "));
    const suffix = result.rows.length > 50 ? t.cdb_ai_result_truncated(result.rows.length) : "";
    return `${header}\n${rows.join("\n")}${suffix}`;
  };

  const buildSystemPrompt = () => {
    const dbDescriptions = databases.map((db) => {
      const tableList = db.tables.map((t) => t.name).join(", ");
      return `[${db.name}] Type: ${db.db_type.toUpperCase()}, Database: ${db.database}, Schema: ${db.schema}\n  Tables: ${tableList || "None"}`;
    }).join("\n");

    const maxSteps = maxStepsRef.current;
    const maxStepsStr = maxSteps >= 9999 ? (locale === "zh-TW" ? "不限次數" : "unlimited") : String(maxSteps);
    return t.cdb_ai_system_prompt(dbDescriptions, maxStepsStr);
  };

  const findDb = (alias: string): ConnectedDb | undefined => {
    return databases.find((db) =>
      db.name === alias ||
      db.name.toLowerCase() === alias.toLowerCase() ||
      db.database === alias ||
      db.database.toLowerCase() === alias.toLowerCase()
    );
  };

  const updateLastMsg = (updater: (m: Message) => Message) => {
    setMessages((prev) => {
      const copy = [...prev];
      copy[copy.length - 1] = updater(copy[copy.length - 1]);
      return copy;
    });
  };

  const send = async () => {
    if (!input.trim() || sending) return;
    const userMsg = input.trim();
    setInput("");
    setSending(true);
    stoppedRef.current = false;

    const historyForAi = messages
      .filter((m) => !m.agentRunning)
      .map((m) => ({ role: m.role, content: m.text }));

    const userMessage: Message = { role: "user", text: userMsg };
    const assistantMessage: Message = {
      role: "assistant",
      text: "",
      steps: [],
      agentRunning: true,
      agentStepLabel: t.cdb_ai_thinking_n(1, maxStepsRef.current >= 9999 ? "" : "/" + maxStepsRef.current),
    };

    const nextMessages = [...messages, userMessage, assistantMessage];
    setMessages(nextMessages);

    const loopHistory: { role: "user" | "assistant"; content: string }[] = [
      ...historyForAi,
      { role: "user", content: userMsg },
    ];

    try {
      let stepCount = 0;
      const maxSteps = maxStepsRef.current;
      let lastExecutedSql = "";

      while (stepCount < maxSteps && !stoppedRef.current) {
        const stepLabel = t.cdb_ai_thinking_n(stepCount + 1, maxSteps >= 9999 ? "" : "/" + maxSteps);
        updateLastMsg((m) => ({ ...m, agentStepLabel: stepLabel }));

        const lastUserContent = loopHistory[loopHistory.length - 1].content;
        const aiResult = await aiChat(
          [{ role: "system" as const, content: buildSystemPrompt() }, ...loopHistory.slice(0, -1), { role: "user" as const, content: lastUserContent }],
          "crossdb",
          selectedProviderId || undefined,
          false,
          locale,
        );
        const reply = aiResult.content ?? "";

        if (import.meta.env.DEV) {
          console.log("[CrossDB] AI reply:", JSON.stringify(reply));
        }

        const parsed = extractCrossDbSql(reply);
        if (import.meta.env.DEV) {
          console.log("[CrossDB] parsed:", parsed);
        }

        if (stoppedRef.current) break;

        if (!parsed) {
          updateLastMsg((m) => ({
            ...m,
            text: reply,
            agentRunning: false,
            agentStepLabel: undefined,
          }));
          if (sendRemoteResponse) sendRemoteResponse(reply);
          break;
        }

        if (parsed.sql === lastExecutedSql) {
          updateLastMsg((m) => ({ ...m, agentStepLabel: t.db_ai_summarizing_label }));
          const summaryResult1 = await aiChat(
            [{ role: "system" as const, content: buildSystemPrompt() }, ...loopHistory, { role: "user" as const, content: t.cdb_ai_summarizing_prompt }],
            "crossdb",
            selectedProviderId || undefined,
            false,
            locale,
          );
          const summary = summaryResult1.content ?? "";
          updateLastMsg((m) => ({
            ...m,
            text: summary,
            agentRunning: false,
            agentStepLabel: undefined,
          }));
          if (sendRemoteResponse) sendRemoteResponse(summary);
          break;
        }

        let targetDb: ConnectedDb | undefined;
        if (parsed.alias === null) {
          if (databases.length === 1) {
            targetDb = databases[0];
          } else {
            const dbsList = databases.map((d) => d.name).join(", ");
            const errMsg = t.cdb_ai_db_missing_err(parsed.alias || "", dbsList);
            loopHistory.push({ role: "assistant", content: reply });
            loopHistory.push({ role: "user", content: errMsg });
            stepCount++;
            continue;
          }
        } else {
          targetDb = findDb(parsed.alias);
        }

        if (!targetDb) {
          const dbsList = databases.map((d) => d.name).join(", ");
          const errMsg = t.cdb_ai_db_not_found_err(parsed.alias || "", dbsList);
          loopHistory.push({ role: "assistant", content: reply });
          loopHistory.push({ role: "user", content: errMsg });
          stepCount++;
          continue;
        }

        const stepIndex = stepCount;
        updateLastMsg((m) => ({
          ...m,
          steps: [...(m.steps ?? []), { targetDb: targetDb.name, sql: parsed.sql, executing: true, collapsed: false }],
        }));

        let result: QueryResult;
        try {
          result = await dbExecuteQuery(targetDb.id, parsed.sql, targetDb.schema || undefined);
        } catch (e: unknown) {
          result = { columns: [], rows: [], affected_rows: null, execution_time_ms: 0, error: String(e) };
        }

        if (stoppedRef.current) break;

        updateLastMsg((m) => ({
          ...m,
          steps: (m.steps ?? []).map((s, i) =>
            i === stepIndex ? { ...s, result, executing: false, collapsed: false } : s
          ),
        }));

        lastExecutedSql = parsed.sql;

        loopHistory.push({ role: "assistant", content: reply });

        const feedbackContent = result.error
          ? t.cdb_ai_execution_err(targetDb.name, result.error)
          : t.cdb_ai_execution_ok(targetDb.name, formatResultForAi(result));

        loopHistory.push({ role: "user", content: feedbackContent });

        stepCount++;

        if (stepCount >= maxSteps) {
          updateLastMsg((m) => ({ ...m, agentStepLabel: t.db_ai_summarizing_label }));
          const summaryResult2 = await aiChat(
            [{ role: "system" as const, content: buildSystemPrompt() }, ...loopHistory, { role: "user" as const, content: t.cdb_ai_summarizing_prompt }],
            "crossdb",
            selectedProviderId || undefined,
            false,
            locale,
          );
          const summary = summaryResult2.content ?? "";
          updateAndPersistCross((m) => ({
            ...m,
            text: summary,
            agentRunning: false,
            agentStepLabel: undefined,
          }));
          if (sendRemoteResponse) sendRemoteResponse(summary);
        }
      }

      if (stoppedRef.current) {
        updateLastMsg((m) => ({
          ...m,
          text: m.text || t.cdb_ai_stopped,
          agentRunning: false,
          agentStepLabel: undefined,
        }));
      }
    } catch (e: unknown) {
      const errText = formatAiError(normalizeAiError(e));
      setMessages((prev) => {
        const copy = [...prev];
        copy[copy.length - 1] = {
          ...copy[copy.length - 1],
          text: t.cdb_ai_error(errText),
          agentRunning: false,
          agentStepLabel: undefined,
        };
        return copy;
      });
    } finally {
      setSending(false);
    }
  };

  const updateAndPersistCross = (updater: (m: Message) => Message) => {
    setMessages((prev) => {
      const copy = [...prev];
      copy[copy.length - 1] = updater(copy[copy.length - 1]);
      return copy;
    });
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
      saveSessions(next);
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

  const formatDate = (ts: number): string => {
    const d = new Date(ts);
    return d.toLocaleDateString(locale === "zh-TW" ? "zh-TW" : "en-US", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
  };

  return (
    <div className="crossdb-chat">
      {/* History side panel */}
      {historyOpen && (
        <div className="crossdb-history-panel">
          <div className="crossdb-history-panel__header">
            <span className="crossdb-history-panel__title">{t.cdb_ai_history_title}</span>
            <button className="crossdb-history-panel__new-btn" onClick={newChat}>{t.cdb_ai_history_new_btn}</button>
          </div>
          <div className="crossdb-history-panel__list">
            {sessions.length === 0 && (
              <div className="crossdb-history-panel__empty">{t.cdb_ai_history_empty}</div>
            )}
            {[...sessions].reverse().map((s) => (
              <div
                key={s.id}
                className={`crossdb-history-panel__item${currentSessionIdRef.current === s.id ? " crossdb-history-panel__item--active" : ""}`}
                onClick={() => loadSession(s)}
              >
                <div className="crossdb-history-panel__item-content">
                  <div className="crossdb-history-panel__item-title">{s.title}</div>
                  <div className="crossdb-history-panel__item-date">{formatDate(s.savedAt)}</div>
                </div>
                <button
                  className="crossdb-history-panel__item-del"
                  onClick={(e) => deleteSession(s.id, e)}
                  title={t.cdb_ai_delete_tooltip}
                >×</button>
              </div>
            ))}
          </div>
        </div>
      )}
      <div className="crossdb-chat__main">
      <div ref={messagesContainerRef} className="crossdb-chat__messages">
        {messages.length === 0 && (
          <div className="crossdb-chat__welcome">
            <h3>{t.cdb_welcome_title}</h3>
            <p>{t.cdb_ai_welcome_desc(databases.length)}</p>
            <div className="crossdb-chat__db-list">
              {databases.map((db) => (
                <div key={db.id} className="crossdb-chat__db-item">
                  <span className="crossdb-chat__db-badge">{db.db_type.toUpperCase()}</span>
                  <span>{db.name}</span>
                  <span className="crossdb-chat__db-detail">{t.cdb_ai_db_detail(db.tables.length)}</span>
                </div>
              ))}
            </div>
          </div>
        )}
        {messages.map((m, i) => (
          <div key={i} className={`crossdb-chat__bubble crossdb-chat__bubble--${m.role}`}>
            {m.role === "user" ? (
              <div className="crossdb-chat__user-text">{m.text}</div>
            ) : (
              <div className="crossdb-chat__assistant">
                {(m.steps ?? []).map((step, si) => (
                  <div key={si} className="crossdb-chat__step">
                    <div
                      className="crossdb-chat__step-header"
                      onClick={() => {
                        updateLastMsg((msg) => ({
                          ...msg,
                          steps: (msg.steps ?? []).map((s, j) =>
                            j === si ? { ...s, collapsed: !s.collapsed } : s
                          ),
                        }));
                      }}
                    >
                      <span className="crossdb-chat__step-db">[{step.targetDb}]</span>
                      <code className="crossdb-chat__step-sql">{step.sql.length > 80 ? step.sql.slice(0, 80) + "..." : step.sql}</code>
                      {step.executing && <span className="crossdb-chat__step-status">⏳</span>}
                      {step.result && !step.result.error && <span className="crossdb-chat__step-status crossdb-chat__step-ok">{t.cdb_ai_step_result_ok(step.result.rows.length)}</span>}
                      {step.result?.error && <span className="crossdb-chat__step-status crossdb-chat__step-err">✗</span>}
                    </div>
                    {!step.collapsed && step.result && (
                      <div className="crossdb-chat__step-result">
                        {step.result.error ? (
                          <div className="crossdb-chat__step-error">{step.result.error}</div>
                        ) : step.result.columns.length > 0 ? (
                          <table className="crossdb-chat__table">
                            <thead>
                              <tr>
                                {step.result.columns.map((col) => <th key={col}>{col}</th>)}
                              </tr>
                            </thead>
                            <tbody>
                              {step.result.rows.slice(0, 20).map((row, ri) => (
                                <tr key={ri}>
                                  {row.map((cell, ci) => (
                                    <td key={ci} className={cell === null ? "null-cell" : ""}>{cell === null ? "NULL" : String(cell)}</td>
                                  ))}
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        ) : (
                          <div className="crossdb-chat__step-meta">
                            {step.result.affected_rows !== null ? t.cdb_ai_result_affected(step.result.affected_rows) : t.cdb_ai_result_empty}
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                ))}
                {m.agentRunning && m.agentStepLabel && (
                  <div className="crossdb-chat__thinking">{m.agentStepLabel}</div>
                )}
                {m.text && !m.agentRunning && (
                  <CrossDbAssistantAnswer text={m.text} />
                )}
                {m.text && m.agentRunning && (
                  <div className="crossdb-chat__answer">
                    <MarkdownText text={extractResponseText(unescapeNewlines(m.text)).replace(/<cmd>([\/\s\S]*?)<\/cmd>/gi, (_m, c) => `\`\`\`\n${c.trim()}\n\`\`\``)} />
                  </div>
                )}
              </div>
            )}
          </div>
        ))}
      </div>

      <div className="crossdb-chat__input-area">
        <div className="crossdb-chat__input-toolbar">
          <button
            type="button"
            className={`crossdb-chat__history-btn${historyOpen ? " crossdb-chat__history-btn--active" : ""}`}
            onClick={() => setHistoryOpen((o) => !o)}
            title={t.cdb_ai_history_title}
          >{t.db_ai_btn_history}</button>
          <ModelPickerButton
            providers={providers}
            selectedId={selectedProviderId}
            onChange={setSelectedProviderId}
          />
        </div>
        <div className="crossdb-chat__input-row">
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
            placeholder={t.cdb_ai_input_placeholder}
            className="crossdb-chat__textarea"
            rows={2}
          />
          {sending ? (
            <button className="crossdb-chat__stop-btn aiterm-btn aiterm-btn--danger-solid aiterm-btn--icon" onClick={stop} title={t.cdb_ai_btn_stop}>■</button>
          ) : (
            <button
              id="crossdb-ai-send-btn"
              className="crossdb-chat__send-btn aiterm-btn aiterm-btn--primary aiterm-btn--icon"
              onClick={send}
              disabled={!input.trim()}
              title={t.cdb_ai_btn_send}
            >
              ▲
            </button>
          )}
        </div>
      </div>
      </div>
    </div>
  );
}

function CrossDbAssistantAnswer({ text }: { text: string }) {
  const { t } = useLocale();
  const [copied, setCopied] = useState(false);
  const cleaned = extractResponseText(unescapeNewlines(text)).replace(/<cmd>([\/\s\S]*?)<\/cmd>/gi, (_m, c) => `\`\`\`\n${c.trim()}\n\`\`\``);
  const handleCopy = () => {
    void navigator.clipboard.writeText(cleaned).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };
  return (
    <div className="crossdb-chat__answer crossdb-chat__answer--copyable">
      <MarkdownText text={cleaned} />
      <button
        type="button"
        className={`crossdb-chat__copy-btn${copied ? " crossdb-chat__copy-btn--copied" : ""}`}
        onClick={handleCopy}
        title={t.cdb_ai_copy_tooltip}
      >{copied ? "✓" : "⎘"}</button>
    </div>
  );
}
