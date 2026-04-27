import { useState, useEffect, useRef } from "react";
import { dbExecuteQuery, type QueryResult } from "../../ipc/db";
import { aiChat, formatAiError, type AiError } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { getConfig } from "../../ipc/config";
import { extractResponseText, unescapeNewlines, MarkdownText } from "../../lib/markdown";
import type { ConnectedDb } from "./index";

interface Props {
  databases: ConnectedDb[];
  sendRemoteResponse?: (text: string) => void;
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

function extractCrossDbSql(text: string): { alias: string; sql: string } | null {
  // Match: ```sql [DB-Name]\nSQL\n```
  const match = text.match(/```sql\s+\[(.+?)\]\s*\n([\s\S]*?)```/i);
  if (match) return { alias: match[1].trim(), sql: match[2].trim() };

  // Fallback: ```sql\n[DB-Name]\nSQL\n```
  const fallback = text.match(/```sql\s*\n\[(.+?)\]\s*\n([\s\S]*?)```/i);
  if (fallback) return { alias: fallback[1].trim(), sql: fallback[2].trim() };

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
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState<string>("");
  const maxStepsRef = useRef<number>(5);
  const bottomRef = useRef<HTMLDivElement>(null);
  const stoppedRef = useRef(false);

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
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const buildSystemPrompt = () => {
    const dbDescriptions = databases.map((db) => {
      const tableList = db.tables.map((t) => t.name).join(", ");
      return `[${db.name}] 類型: ${db.db_type.toUpperCase()}, Database: ${db.database}, Schema: ${db.schema}\n  資料表: ${tableList || "（無）"}`;
    }).join("\n");

    const maxSteps = maxStepsRef.current;
    return `你是一個跨資料庫 Agent，可以同時查詢多個資料庫來回答使用者問題。

可用資料庫：
${dbDescriptions}

重要規則（必須嚴格遵守）：
1. 需要查詢時，使用以下格式指定目標資料庫（第一行必須用方括號標注資料庫名稱）：
\`\`\`sql [資料庫名稱]
SELECT * FROM ...
\`\`\`
2. 每次只查詢一個資料庫的一條 SQL，但你可以根據前次查詢結果，決定下一步查詢哪個庫
3. 不同資料庫的 SQL 方言可能不同（PostgreSQL vs MySQL vs MSSQL 等），請注意語法差異
4. 已收集足夠資料時，直接用繁體中文給出最終彙整答案，回應中不包含任何 SQL 或程式碼區塊
5. 最多執行 ${maxSteps >= 9999 ? "不限次數" : maxSteps} 次查詢`;
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
      agentStepLabel: "思考中...",
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

        const parsed = extractCrossDbSql(reply);

        if (!parsed) {
          // No SQL → final answer
          updateLastMsg((m) => ({
            ...m,
            text: reply,
            agentRunning: false,
            agentStepLabel: undefined,
          }));
          if (sendRemoteResponse) sendRemoteResponse(reply);
          break;
        }

        const targetDb = findDb(parsed.alias);
        if (!targetDb) {
          const errMsg = `找不到名為「${parsed.alias}」的資料庫。可用：${databases.map((d) => d.name).join(", ")}`;
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

        loopHistory.push({ role: "assistant", content: reply });
        loopHistory.push({
          role: "user",
          content: `[${targetDb.name}] SQL 執行結果：\n\`\`\`\n${formatResultForAi(result)}\n\`\`\`\n\n請繼續分析或給出最終答案。`,
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
          updateLastMsg((m) => ({
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
          text: m.text || "（已停止）",
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
          text: `錯誤：${errText}`,
          agentRunning: false,
          agentStepLabel: undefined,
        };
        return copy;
      });
    } finally {
      setSending(false);
    }
  };

  const stop = () => { stoppedRef.current = true; };

  return (
    <div className="crossdb-chat">
      <div className="crossdb-chat__messages">
        {messages.length === 0 && (
          <div className="crossdb-chat__welcome">
            <h3>🔗 跨資料庫 AI 查詢</h3>
            <p>已連接 {databases.length} 個資料庫。請描述您想查詢的問題，AI 將自動路由到正確的資料庫...</p>
            <div className="crossdb-chat__db-list">
              {databases.map((db) => (
                <div key={db.id} className="crossdb-chat__db-item">
                  <span className="crossdb-chat__db-badge">{db.db_type.toUpperCase()}</span>
                  <span>{db.name}</span>
                  <span className="crossdb-chat__db-detail">{db.tables.length} 個資料表</span>
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
                      {step.result && !step.result.error && <span className="crossdb-chat__step-status crossdb-chat__step-ok">✓ {step.result.rows.length} 列</span>}
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
                            {step.result.affected_rows !== null ? `${step.result.affected_rows} 列受影響` : "無結果"}
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                ))}
                {m.agentRunning && m.agentStepLabel && (
                  <div className="crossdb-chat__thinking">{m.agentStepLabel}</div>
                )}
                {m.text && (
                  <div className="crossdb-chat__answer">
                    <MarkdownText text={extractResponseText(unescapeNewlines(m.text))} />
                  </div>
                )}
              </div>
            )}
          </div>
        ))}
        <div ref={bottomRef} />
      </div>

      <div className="crossdb-chat__input-area">
        <div className="crossdb-chat__input-row">
          {providers.length > 1 && (
            <select
              value={selectedProviderId}
              onChange={(e) => setSelectedProviderId(e.target.value)}
              className="crossdb-chat__provider-select"
            >
              {providers.map((p) => (
                <option key={p.id} value={p.id}>{p.display_name}</option>
              ))}
            </select>
          )}
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
            placeholder="描述跨資料庫查詢需求..."
            className="crossdb-chat__textarea"
            rows={2}
          />
          {sending ? (
            <button className="crossdb-chat__stop-btn" onClick={stop}>■ 停止</button>
          ) : (
            <button
              id="crossdb-ai-send-btn"
              className="crossdb-chat__send-btn"
              onClick={send}
              disabled={!input.trim()}
            >
              送出
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
