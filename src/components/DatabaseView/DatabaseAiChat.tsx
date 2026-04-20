import { useState, useEffect, useRef } from "react";
import { dbListTables, dbExecuteQuery, type TableInfo, type QueryResult } from "../../ipc/db";
import { aiChat, formatAiError, type AiError } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { getConfig } from "../../ipc/config";

interface Props {
  connectionId: string;
  schema: string;
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

  return null;
}

function formatResultForAi(result: QueryResult): string {
  if (result.error) return `錯誤：${result.error}`;
  if (result.affected_rows !== null) return `執行成功，${result.affected_rows} 列受影響`;
  if (result.columns.length === 0) return "無結果";
  const header = result.columns.join(" | ");
  const rows = result.rows.slice(0, 50).map((r) => r.map((c) => (c === null ? "NULL" : String(c))).join(" | "));
  const suffix = result.rows.length > 50 ? `\n...（共 ${result.rows.length} 列，只顯示前 50 列）` : "";
  return `${header}\n${rows.join("\n")}${suffix}`;
}

function formatDate(ts: number): string {
  const d = new Date(ts);
  return d.toLocaleDateString("zh-TW", { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
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

export function DatabaseAiChat({ connectionId, schema }: Props) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [tables, setTables] = useState<TableInfo[]>([]);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState<string>("");
  const [historyOpen, setHistoryOpen] = useState(false);
  const [sessions, setSessions] = useState<SavedSession[]>([]);
  const maxStepsRef = useRef<number>(5);
  const bottomRef = useRef<HTMLDivElement>(null);
  const stoppedRef = useRef(false);
  const currentSessionIdRef = useRef<string | null>(null);

  useEffect(() => {
    if (schema) {
      dbListTables(connectionId, schema).then(setTables).catch(console.error);
    }
  }, [connectionId, schema]);

  useEffect(() => {
    getConfig().then((cfg) => {
      // 0 = unlimited; use a large number internally
      maxStepsRef.current = cfg.max_agent_steps === 0 ? 9999 : (cfg.max_agent_steps ?? 5);
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
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const buildSystemPrompt = () => {
    const tableList = tables.map((t) => t.name).join(", ");
    const maxSteps = maxStepsRef.current;
    return `你是一個資料庫 Agent，可執行多次 SQL 查詢來回答使用者問題。
Schema：「${schema}」，可用資料表：${tableList || "（載入中）"}。

重要規則（必須嚴格遵守）：
1. 需要查詢資料時，只輸出 \`\`\`sql\n你的SQL\n\`\`\`，不要有任何其他格式（不要 JSON、不要 thought 欄位）
2. 每次只提供一條 SQL
3. 已收集足夠資料時，直接用繁體中文給出最終答案，回應中不包含任何 SQL 或程式碼區塊
4. 最多執行 ${maxSteps >= 9999 ? "不限次數" : maxSteps} 次查詢`;
  };

  const updateLastMsg = (updater: (m: Message) => Message) => {
    setMessages((prev) => {
      const copy = [...prev];
      copy[copy.length - 1] = updater(copy[copy.length - 1]);
      return copy;
    });
  };

  /** Auto-save/update the current session to localStorage */
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

    // Start a new session if this is the first message
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
      agentStepLabel: "思考中...",
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
        const stepLabel = maxSteps >= 9999
          ? `思考中... (步驟 ${stepCount + 1})`
          : `思考中... (步驟 ${stepCount + 1}/${maxSteps})`;
        updateLastMsg((m) => ({ ...m, agentStepLabel: stepLabel }));

        const lastUserContent = loopHistory[loopHistory.length - 1].content;
        const reply = await aiChat(
          lastUserContent,
          buildSystemPrompt(),
          loopHistory.slice(0, -1),
          selectedProviderId || undefined,
        );

        if (stoppedRef.current) break;

        const sql = extractSql(reply);

        if (!sql) {
          updateAndPersist((m) => ({
            ...m,
            text: reply,
            agentRunning: false,
            agentStepLabel: undefined,
          }));
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
          content: `SQL 執行結果：\n\`\`\`\n${formatResultForAi(result)}\n\`\`\`\n\n請繼續分析或給出最終答案。`,
        });

        stepCount++;

        if (stepCount >= maxSteps) {
          updateLastMsg((m) => ({ ...m, agentStepLabel: "整理答案中..." }));
          const summary = await aiChat(
            "請根據以上查詢結果，用繁體中文給出最終完整答案，不要再提供 SQL。",
            buildSystemPrompt(),
            loopHistory,
            selectedProviderId || undefined,
          );
          updateAndPersist((m) => ({
            ...m,
            text: summary,
            agentRunning: false,
            agentStepLabel: undefined,
          }));
        }
      }

      if (stoppedRef.current) {
        updateAndPersist((m) => ({
          ...m,
          text: m.text || "（已停止）",
          agentRunning: false,
          agentStepLabel: undefined,
        }));
      }

      void finalMessages; // suppress unused warning
    } catch (e: unknown) {
      setMessages((prev) => {
        const copy = [...prev];
        copy[copy.length - 1] = {
          ...copy[copy.length - 1],
          text: `錯誤：${errorText(e)}`,
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

  return (
    <div style={{ display: "flex", height: "100%", flex: 1, minWidth: 0, position: "relative" }}>
      {/* History side panel */}
      {historyOpen && (
        <div style={{
          width: 240, borderRight: "1px solid #1e1e1e", background: "#0e0e0e",
          display: "flex", flexDirection: "column", flexShrink: 0,
        }}>
          <div style={{ padding: "8px 12px", borderBottom: "1px solid #1e1e1e", display: "flex", alignItems: "center", justifyContent: "space-between" }}>
            <span style={{ fontSize: 11, color: "#555", textTransform: "uppercase", letterSpacing: "0.05em" }}>對話歷史</span>
            <button
              onClick={newChat}
              style={{ background: "#1a2a1e", border: "1px solid #2d4a35", color: "#4ade80", fontSize: 10, borderRadius: 4, padding: "2px 8px", cursor: "pointer" }}
            >
              ＋ 新對話
            </button>
          </div>
          <div style={{ flex: 1, overflowY: "auto" }}>
            {sessions.length === 0 && (
              <div style={{ color: "#444", fontSize: 12, padding: "16px 12px" }}>尚無歷史記錄</div>
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
                  title="刪除此對話"
                  style={{ background: "transparent", border: "none", color: "#444", fontSize: 14, cursor: "pointer", padding: "2px 5px", borderRadius: 3, flexShrink: 0, lineHeight: 1 }}
                  onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.color = "#f87171"; (e.currentTarget as HTMLButtonElement).style.background = "#2a1a1a"; }}
                  onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.color = "#444"; (e.currentTarget as HTMLButtonElement).style.background = "transparent"; }}
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Main chat area */}
      <div style={{ display: "flex", flexDirection: "column", flex: 1, minWidth: 0 }}>
        <div style={{ flex: 1, overflowY: "auto", padding: "12px 16px", display: "flex", flexDirection: "column", gap: 12 }}>
          {messages.length === 0 && (
            <div style={{ color: "#555", fontSize: 13, padding: "20px 0" }}>
              用自然語言描述你想查詢的資料，例如：「查詢最近 10 筆訂單」
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
          <div ref={bottomRef} />
        </div>

        {/* Toolbar: provider selector + history toggle */}
        <div style={{ borderTop: "1px solid #1e1e1e", padding: "6px 12px", display: "flex", alignItems: "center", gap: 8, background: "#111" }}>
          <span style={{ fontSize: 11, color: "#555", flexShrink: 0 }}>模型</span>
          <select
            value={selectedProviderId}
            onChange={(e) => setSelectedProviderId(e.target.value)}
            style={{
              background: "#0c0c0c", border: "1px solid #2a2a2a", color: "#aaa",
              borderRadius: 4, padding: "2px 6px", fontSize: 11, cursor: "pointer", outline: "none",
            }}
          >
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.display_name} ({p.model}){p.is_default ? " ★" : ""}
              </option>
            ))}
            {providers.length === 0 && <option value="">（未設定）</option>}
          </select>
          <div style={{ flex: 1 }} />
          <button
            onClick={() => setHistoryOpen((o) => !o)}
            title="對話歷史"
            style={{
              background: historyOpen ? "#1a2030" : "transparent",
              border: "1px solid " + (historyOpen ? "#3b5bdb" : "#2a2a2a"),
              color: historyOpen ? "#74b9ff" : "#555",
              borderRadius: 4, padding: "2px 8px", fontSize: 11, cursor: "pointer",
            }}
          >
            🕐 歷史
          </button>
          {messages.length > 0 && (
            <button
              onClick={newChat}
              title="開始新對話"
              style={{
                background: "transparent", border: "1px solid #2a2a2a", color: "#555",
                borderRadius: 4, padding: "2px 8px", fontSize: 11, cursor: "pointer",
              }}
            >
              ＋ 新對話
            </button>
          )}
        </div>

        {/* Input bar */}
        <div style={{ borderTop: "1px solid #1e1e1e", padding: "10px 12px", display: "flex", gap: 8, alignItems: "flex-end", background: "#111" }}>
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); }
            }}
            placeholder="用自然語言描述查詢... (Enter 送出)"
            rows={2}
            style={{
              flex: 1, background: "#0c0c0c", border: "1px solid #2a2a2a", color: "#e6e6e6",
              borderRadius: 6, padding: "8px 10px", fontSize: 13, resize: "none", outline: "none",
              fontFamily: "inherit",
            }}
          />
          {sending ? (
            <button
              onClick={stop}
              style={{ background: "#3a1a1a", border: "1px solid #f87171", color: "#f87171", borderRadius: 6, padding: "8px 14px", cursor: "pointer", fontSize: 12 }}
            >
              ■ 停止
            </button>
          ) : (
            <button
              onClick={send}
              disabled={!input.trim()}
              style={{ background: "#1e3a2e", border: "1px solid #34d399", color: "#34d399", borderRadius: 6, padding: "8px 14px", cursor: "pointer", fontSize: 12 }}
            >
              ✨ 送出
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function MessageBubble({ msg, onToggleStep }: { msg: Message; onToggleStep: (i: number) => void }) {
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
              步驟 {i + 1}：{step.sql}
            </span>
            <span>{step.collapsed ? "▶" : "▼"}</span>
          </button>
          {!step.collapsed && (
            <div style={{ borderTop: "1px solid #1e1e1e", padding: "8px 10px" }}>
              <div style={{ fontFamily: "monospace", fontSize: 11, color: "#34d399", marginBottom: 6, whiteSpace: "pre-wrap" }}>
                {step.sql}
              </div>
              {step.executing && <div style={{ color: "#888", fontSize: 11 }}>執行中...</div>}
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

      {msg.text && (
        <div style={{
          background: "#1a1a1a", border: "1px solid #2a2a2a",
          borderRadius: 8, padding: "8px 12px", fontSize: 13, color: "#e6e6e6",
          whiteSpace: "pre-wrap",
        }}>
          {msg.text}
        </div>
      )}
    </div>
  );
}

function ResultInline({ result }: { result: QueryResult }) {
  if (result.error) {
    return <div style={{ color: "#f87171", fontSize: 11 }}>✗ {result.error}</div>;
  }
  if (result.affected_rows !== null) {
    return <div style={{ color: "#34d399", fontSize: 11 }}>✓ {result.affected_rows} 列受影響 ({result.execution_time_ms}ms)</div>;
  }
  if (result.columns.length === 0) return null;
  return (
    <div style={{ overflowX: "auto" }}>
      <div style={{ color: "#555", fontSize: 10, marginBottom: 4 }}>{result.rows.length} 列 · {result.execution_time_ms}ms</div>
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
            <tr><td colSpan={result.columns.length} style={{ padding: "3px 8px", color: "#444", fontSize: 10 }}>... 還有 {result.rows.length - 20} 列</td></tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
