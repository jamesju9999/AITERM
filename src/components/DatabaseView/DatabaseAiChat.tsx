import { useState, useEffect, useRef } from "react";
import { dbListTables, dbExecuteQuery, type TableInfo, type QueryResult } from "../../ipc/db";
import { aiChat } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";

const MAX_STEPS = 5;

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
  agentStepLabel?: string; // e.g. "思考中... (步驟 2/5)"
}

function extractSql(text: string): string | null {
  // Primary: ```sql ... ```
  const sqlBlock = text.match(/```sql\s*([\s\S]*?)```/i);
  if (sqlBlock) return sqlBlock[1].trim();

  // Fallback: ```json { "sql": "..." } ``` (some models use structured output)
  const jsonBlock = text.match(/```(?:json)?\s*(\{[\s\S]*?\})\s*```/i);
  if (jsonBlock) {
    try {
      const parsed = JSON.parse(jsonBlock[1]);
      if (typeof parsed.sql === "string" && parsed.sql.trim()) return parsed.sql.trim();
    } catch { /* not valid JSON */ }
  }

  // Fallback: bare JSON object with a "sql" key
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

export function DatabaseAiChat({ connectionId, schema }: Props) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [tables, setTables] = useState<TableInfo[]>([]);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState<string>("");
  const bottomRef = useRef<HTMLDivElement>(null);
  const stoppedRef = useRef(false);

  useEffect(() => {
    if (schema) {
      dbListTables(connectionId, schema).then(setTables).catch(console.error);
    }
  }, [connectionId, schema]);

  useEffect(() => {
    listProviders().then((list) => {
      setProviders(list);
      const def = list.find((p) => p.is_default);
      if (def) setSelectedProviderId(def.id);
    }).catch(console.error);
  }, []);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const buildSystemPrompt = () => {
    const tableList = tables.map((t) => t.name).join(", ");
    return `你是一個資料庫 Agent，可執行多次 SQL 查詢來回答使用者問題。
Schema：「${schema}」，可用資料表：${tableList || "（載入中）"}。

重要規則（必須嚴格遵守）：
1. 需要查詢資料時，只輸出 \`\`\`sql\n你的SQL\n\`\`\`，不要有任何其他格式（不要 JSON、不要 thought 欄位）
2. 每次只提供一條 SQL
3. 已收集足夠資料時，直接用繁體中文給出最終答案，回應中不包含任何 SQL 或程式碼區塊
4. 最多執行 ${MAX_STEPS} 次查詢`;
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

    // Conversation history for AI (role+text only, excluding intermediate step messages)
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
    setMessages((prev) => [...prev, userMessage, assistantMessage]);

    // Running AI loop conversation: prior history + new user message, then interleaved results
    // Structure: [...historyForAi, { role: "user", userMsg }, { role: "assistant", reply }, { role: "user", result }, ...]
    const loopHistory: { role: "user" | "assistant"; content: string }[] = [
      ...historyForAi,
      { role: "user", content: userMsg },
    ];

    try {
      let stepCount = 0;

      while (stepCount < MAX_STEPS && !stoppedRef.current) {
        const stepLabel = `思考中... (步驟 ${stepCount + 1}/${MAX_STEPS})`;
        updateLastMsg((m) => ({ ...m, agentStepLabel: stepLabel }));

        // Last item in loopHistory is always a "user" message; pass it as the message argument
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
          // Final answer — no SQL in response
          updateLastMsg((m) => ({
            ...m,
            text: reply,
            agentRunning: false,
            agentStepLabel: undefined,
          }));
          break;
        }

        // Add this step
        const stepIndex = stepCount;
        updateLastMsg((m) => ({
          ...m,
          steps: [...(m.steps ?? []), { sql, executing: true, collapsed: false }],
        }));

        // Execute SQL
        let result: QueryResult;
        try {
          result = await dbExecuteQuery(connectionId, sql);
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

        // Feed result back into conversation
        loopHistory.push({ role: "assistant", content: reply });
        loopHistory.push({
          role: "user",
          content: `SQL 執行結果：\n\`\`\`\n${formatResultForAi(result)}\n\`\`\`\n\n請繼續分析或給出最終答案。`,
        });

        stepCount++;

        if (stepCount >= MAX_STEPS) {
          // Force a final summary
          updateLastMsg((m) => ({ ...m, agentStepLabel: "整理答案中..." }));
          const summary = await aiChat(
            "請根據以上查詢結果，用繁體中文給出最終完整答案，不要再提供 SQL。",
            buildSystemPrompt(),
            loopHistory,
            selectedProviderId || undefined,
          );
          updateLastMsg((m) => ({
            ...m,
            text: summary,
            agentRunning: false,
            agentStepLabel: undefined,
          }));
        }
      }

      if (stoppedRef.current) {
        updateLastMsg((m) => ({
          ...m,
          text: m.text || "（已停止）",
          agentRunning: false,
          agentStepLabel: undefined,
        }));
      }
    } catch (e: unknown) {
      updateLastMsg((m) => ({
        ...m,
        text: `錯誤：${String(e)}`,
        agentRunning: false,
        agentStepLabel: undefined,
      }));
    } finally {
      setSending(false);
    }
  };

  const stop = () => {
    stoppedRef.current = true;
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", flex: 1, minWidth: 0 }}>
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

      {/* Provider selector */}
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

  // Assistant message
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 6, alignItems: "flex-start", maxWidth: "95%" }}>
      {/* Intermediate agent steps */}
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

      {/* Running indicator */}
      {msg.agentRunning && (
        <div style={{ color: "#888", fontSize: 11, padding: "4px 0" }}>
          <span style={{ marginRight: 6 }}>⟳</span>{msg.agentStepLabel}
        </div>
      )}

      {/* Final answer */}
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
