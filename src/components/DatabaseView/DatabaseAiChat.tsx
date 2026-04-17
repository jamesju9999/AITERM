import { useState, useEffect, useRef } from "react";
import { dbListTables, dbExecuteQuery, TableInfo, QueryResult } from "../../ipc/db";
import { aiChat } from "../../ipc/ai";

interface Props {
  connectionId: string;
  schema: string;
}

interface Message {
  role: "user" | "assistant";
  text: string;
  sql?: string;
  result?: QueryResult;
  executing?: boolean;
}

function extractSql(text: string): string | null {
  const m = text.match(/```sql\s*([\s\S]*?)```/i);
  return m ? m[1].trim() : null;
}

export function DatabaseAiChat({ connectionId, schema }: Props) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [tables, setTables] = useState<TableInfo[]>([]);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (schema) {
      dbListTables(connectionId, schema).then(setTables).catch(console.error);
    }
  }, [connectionId, schema]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const buildSystemPrompt = () => {
    const tableList = tables.map((t) => t.name).join(", ");
    return `你是一個資料庫助手，正在連接資料庫 schema「${schema}」。
可用的資料表：${tableList || "（載入中）"}。
請用繁體中文回答，並以 \`\`\`sql ... \`\`\` 格式提供 SQL 語句。`;
  };

  const executeMessageSql = async (msgIndex: number, sql: string, retryCount = 0): Promise<void> => {
    setMessages((prev) =>
      prev.map((m, i) => (i === msgIndex ? { ...m, executing: true } : m))
    );
    const result = await dbExecuteQuery(connectionId, sql);

    if (result.error && retryCount < 2) {
      const retryPrompt = `SQL 執行錯誤：${result.error}\n原始 SQL：${sql}\n請修正 SQL。`;
      setSending(true);
      try {
        const fixResp = await aiChat(retryPrompt, buildSystemPrompt(), []);
        const fixedSql = extractSql(fixResp);
        if (fixedSql) {
          await executeMessageSql(msgIndex, fixedSql, retryCount + 1);
          return;
        }
      } finally {
        setSending(false);
      }
    }

    setMessages((prev) =>
      prev.map((m, i) => i === msgIndex ? { ...m, sql, result, executing: false } : m)
    );
  };

  const send = async () => {
    if (!input.trim() || sending) return;
    const userMsg = input.trim();
    setInput("");
    setSending(true);

    const newMessages: Message[] = [...messages, { role: "user", text: userMsg }];
    setMessages(newMessages);

    try {
      const history = newMessages.map((m) => ({ role: m.role, content: m.text }));
      const reply = await aiChat(userMsg, buildSystemPrompt(), history.slice(0, -1));
      const sql = extractSql(reply);
      const msgIndex = newMessages.length;
      setMessages((prev) => [...prev, { role: "assistant", text: reply, sql: sql ?? undefined }]);
      if (sql) {
        await executeMessageSql(msgIndex, sql);
      }
    } catch (e: unknown) {
      setMessages((prev) => [...prev, { role: "assistant", text: `錯誤：${String(e)}` }]);
    } finally {
      setSending(false);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ flex: 1, overflowY: "auto", padding: "12px 16px", display: "flex", flexDirection: "column", gap: 12 }}>
        {messages.length === 0 && (
          <div style={{ color: "#555", fontSize: 13, padding: "20px 0" }}>
            用自然語言描述你想查詢的資料，例如：「查詢最近 10 筆訂單」
          </div>
        )}
        {messages.map((msg, i) => (
          <div key={i} style={{ display: "flex", flexDirection: "column", gap: 6, alignItems: msg.role === "user" ? "flex-end" : "flex-start" }}>
            <div style={{
              background: msg.role === "user" ? "#1a2a3e" : "#1a1a1a",
              border: `1px solid ${msg.role === "user" ? "#60a5fa33" : "#2a2a2a"}`,
              borderRadius: 8, padding: "8px 12px", maxWidth: "80%", fontSize: 13, color: "#e6e6e6",
            }}>
              {msg.text}
            </div>
            {msg.sql && (
              <div style={{ background: "#0f0f0f", border: "1px solid #2a2a2a", borderRadius: 6, padding: "8px 12px", fontFamily: "monospace", fontSize: 12, color: "#34d399", maxWidth: "90%" }}>
                {msg.sql}
              </div>
            )}
            {msg.executing && <div style={{ color: "#888", fontSize: 11 }}>執行中...</div>}
            {msg.result && !msg.executing && <ResultInline result={msg.result} />}
          </div>
        ))}
        <div ref={bottomRef} />
      </div>

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
        <button
          onClick={send}
          disabled={sending || !input.trim()}
          style={{ background: "#1e3a2e", border: "1px solid #34d399", color: "#34d399", borderRadius: 6, padding: "8px 14px", cursor: "pointer", fontSize: 12 }}
        >
          {sending ? "..." : "✨ 送出"}
        </button>
      </div>
    </div>
  );
}

function ResultInline({ result }: { result: QueryResult }) {
  if (result.error) {
    return <div style={{ color: "#f87171", fontSize: 11, padding: "4px 12px" }}>✗ {result.error}</div>;
  }
  if (result.affected_rows !== null) {
    return <div style={{ color: "#34d399", fontSize: 11 }}>✓ {result.affected_rows} 列受影響 ({result.execution_time_ms}ms)</div>;
  }
  if (result.columns.length === 0) return null;
  return (
    <div style={{ overflowX: "auto", maxWidth: "90%" }}>
      <div style={{ color: "#888", fontSize: 11, marginBottom: 4 }}>{result.rows.length} 列 · {result.execution_time_ms}ms</div>
      <table style={{ borderCollapse: "collapse", fontSize: 11, fontFamily: "monospace" }}>
        <thead>
          <tr>{result.columns.map((c) => <th key={c} style={{ padding: "3px 8px", color: "#888", borderBottom: "1px solid #222", textAlign: "left", fontWeight: "normal" }}>{c}</th>)}</tr>
        </thead>
        <tbody>
          {result.rows.slice(0, 20).map((row, i) => (
            <tr key={i} style={{ background: i % 2 === 0 ? "transparent" : "#0f0f0f" }}>
              {row.map((cell, j) => (
                <td key={j} style={{ padding: "3px 8px", color: cell === null ? "#444" : "#ccc", borderBottom: "1px solid #111" }}>
                  {cell === null ? "NULL" : String(cell)}
                </td>
              ))}
            </tr>
          ))}
          {result.rows.length > 20 && (
            <tr><td colSpan={result.columns.length} style={{ padding: "4px 8px", color: "#555", fontSize: 11 }}>... 還有 {result.rows.length - 20} 列</td></tr>
          )}
        </tbody>
      </table>
    </div>
  );
}
