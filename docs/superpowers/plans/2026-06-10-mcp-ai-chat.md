# MCP AI Chat Tool Calling — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓 AI Chat 能自動呼叫 MCP 工具，支援 Ollama 本地模型與 Anthropic/OpenAI 雲端模型，並在 chat 中顯示可展開的工具呼叫卡片。

**Architecture:** 以方案 A 實作：`AiPanel` 切換為 `useMcpChat`（已有 agent loop）；為 Ollama 實作原生 tool calling；對 `Unsupported` provider 加入 system prompt 注入 fallback；`MessageList` 加入工具卡片渲染。

**Tech Stack:** Rust/Tokio, reqwest, wiremock（測試）; React 19, TypeScript, Tauri 2 IPC, Vitest + RTL

---

## File Map

| File | 變更類型 | 說明 |
|------|---------|------|
| `src-tauri/src/ai/ollama.rs` | Modify | 加入 `generate_with_tools` 實作 |
| `src-tauri/src/commands/ai.rs` | Modify | 加入 system prompt fallback 分支 |
| `src-tauri/tests/ollama_client.rs` | Modify | 加入 tool calling 測試 |
| `src/hooks/useMcpChat.ts` | Modify | 加入 session history、addMessage、resend、error、isStreaming alias |
| `src/components/AiPanel/MessageList.tsx` | Modify | 支援 McpChatMessage[]、加入工具卡片 |
| `src/components/AiPanel/styles.css` | Modify | 工具卡片 CSS |
| `src/components/AiPanel/index.tsx` | Modify | 切換到 useMcpChat |

---

## Task 1: Ollama generate_with_tools

**Files:**
- Modify: `src-tauri/src/ai/ollama.rs`
- Modify: `src-tauri/tests/ollama_client.rs`

### 背景
Ollama `/api/chat` 支援 `tools` 參數（OpenAI-compatible 格式），但需 `stream: false`。回應的 `message.tool_calls[]` 含工具名稱和參數。

- [ ] **Step 1: 在 `ollama.rs` 加入 request/response 型別**

在 `// ── Request types ─────` 區段下方（約第 132 行），`build_request_body` 函數之前加入：

```rust
// ── Tool calling types ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct OllamaToolRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    tools: Vec<OllamaTool>,
}

#[derive(Serialize)]
struct OllamaTool {
    #[serde(rename = "type")]
    kind: String,
    function: OllamaToolFunction,
}

#[derive(Serialize)]
struct OllamaToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Deserialize)]
struct OllamaToolResponse {
    message: OllamaToolResponseMessage,
}

#[derive(Deserialize)]
struct OllamaToolResponseMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<OllamaResponseToolCall>,
}

#[derive(Deserialize)]
struct OllamaResponseToolCall {
    function: OllamaResponseFunction,
}

#[derive(Deserialize)]
struct OllamaResponseFunction {
    name: String,
    arguments: serde_json::Value,
}
```

- [ ] **Step 2: 在 `OllamaClient` 的 `impl AiProvider` block 加入 `generate_with_tools`**

在 `health_check` 函數（約第 104 行）後面加入：

```rust
async fn generate_with_tools(
    &self,
    req: GenerateRequest,
    tools: Vec<crate::ai::McpToolDefinition>,
    _tx: mpsc::Sender<GenerateChunk>,
) -> Result<crate::ai::GenerateWithToolsResult, AiError> {
    use crate::ai::GenerateWithToolsResult;

    let ollama_tools: Vec<OllamaTool> = tools
        .iter()
        .map(|t| OllamaTool {
            kind: "function".into(),
            function: OllamaToolFunction {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            },
        })
        .collect();

    let mut messages: Vec<OllamaMessage> = Vec::new();
    messages.push(OllamaMessage {
        role: "system".into(),
        content: serde_json::Value::String(req.system_prompt.clone()),
    });
    for m in &req.messages {
        messages.push(OllamaMessage {
            role: m.role.clone(),
            content: m.content.clone(),
        });
    }

    let body = OllamaToolRequest {
        model: self.model.clone(),
        messages,
        stream: false,
        tools: ollama_tools,
    };

    let resp = self
        .client
        .post(self.chat_url())
        .json(&body)
        .send()
        .await
        .map_err(|e| connection_error(&e))?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(AiError::Network {
            message: format!(
                "Ollama http {}: {}",
                status.as_u16(),
                &body_text[..body_text.len().min(200)]
            ),
        });
    }

    let data: OllamaToolResponse = resp
        .json()
        .await
        .map_err(|e| AiError::Network { message: e.to_string() })?;

    if !data.message.tool_calls.is_empty() {
        let calls = data
            .message
            .tool_calls
            .into_iter()
            .enumerate()
            .map(|(i, tc)| crate::ai::AiToolCall {
                id: format!("call_{}", i),
                tool_name: tc.function.name,
                args: tc.function.arguments,
            })
            .collect();
        Ok(GenerateWithToolsResult::ToolCalls(calls))
    } else {
        Ok(GenerateWithToolsResult::Text(data.message.content))
    }
}
```

- [ ] **Step 3: 確認編譯通過**

```bash
cd src-tauri && cargo build 2>&1 | grep -E "^error"
```

Expected: 無 error 輸出

- [ ] **Step 4: 在 `tests/ollama_client.rs` 加入 tool calling 測試**

在檔案末尾加入：

```rust
#[tokio::test]
async fn generate_with_tools_returns_tool_calls() {
    let server = MockServer::start().await;

    let body = r#"{"model":"qwen2.5","message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"brave__search","arguments":{"query":"WWDC 2026"}}}]},"done":true}"#;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OllamaClient::with_base_url("qwen2.5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(4);
    let tools = vec![aiterm_lib::ai::McpToolDefinition {
        name: "brave__search".into(),
        description: "Search the web".into(),
        input_schema: serde_json::json!({ "type": "object", "properties": { "query": { "type": "string" } } }),
    }];

    let result = client.generate_with_tools(req("search WWDC"), tools, tx).await.unwrap();
    match result {
        aiterm_lib::ai::GenerateWithToolsResult::ToolCalls(calls) => {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].tool_name, "brave__search");
            assert_eq!(calls[0].args["query"], "WWDC 2026");
        }
        other => panic!("expected ToolCalls, got {:?}", other),
    }
}

#[tokio::test]
async fn generate_with_tools_returns_text_when_no_tool_calls() {
    let server = MockServer::start().await;

    let body = r#"{"model":"qwen2.5","message":{"role":"assistant","content":"Hello there"},"done":true}"#;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(body),
        )
        .expect(1)
        .mount(&server)
        .await;

    let client = OllamaClient::with_base_url("qwen2.5".into(), server.uri());
    let (tx, _rx) = mpsc::channel::<GenerateChunk>(4);
    let tools = vec![aiterm_lib::ai::McpToolDefinition {
        name: "dummy".into(),
        description: "dummy".into(),
        input_schema: serde_json::json!({}),
    }];

    let result = client.generate_with_tools(req("hello"), tools, tx).await.unwrap();
    match result {
        aiterm_lib::ai::GenerateWithToolsResult::Text(t) => assert_eq!(t, "Hello there"),
        other => panic!("expected Text, got {:?}", other),
    }
}
```

Also add this to the existing imports at the top of the test file:
```rust
use aiterm_lib::ai::GenerateWithToolsResult;
```

- [ ] **Step 5: 執行測試**

```bash
cd src-tauri && cargo test --test ollama_client 2>&1 | tail -15
```

Expected:
```
test generate_with_tools_returns_tool_calls ... ok
test generate_with_tools_returns_text_when_no_tool_calls ... ok
```

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/ai/ollama.rs src-tauri/tests/ollama_client.rs
git commit -m "feat(ollama): implement generate_with_tools for MCP tool calling"
```

---

## Task 2: System prompt fallback for Unsupported providers

**Files:**
- Modify: `src-tauri/src/commands/ai.rs`

### 背景
當 provider 回傳 `Unsupported` 時，把工具描述注入 system prompt，讓模型輸出 `<tool_call>...</tool_call>`，再 parse 回傳。

- [ ] **Step 1: 在 `commands/ai.rs` 加入 helper 函數**

在檔案末尾（`build_chat_prompt` 函數之後）加入：

```rust
/// Build the tool injection suffix for providers that don't support native tool calling.
pub(crate) fn build_tool_prompt_injection(tools: &[crate::ai::McpToolDefinition]) -> String {
    let tools_json: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| serde_json::json!({
            "name": t.name,
            "description": t.description,
            "parameters": t.input_schema,
        }))
        .collect();
    format!(
        "You have access to the following tools. To call a tool, output ONLY a single JSON block \
using this exact format and nothing else before or after it:\n\
<tool_call>{{\"name\":\"<tool_name>\",\"arguments\":{{...}}}}</tool_call>\n\n\
Available tools:\n{}\n\n\
After receiving tool results, continue the conversation naturally in the user's language.",
        serde_json::to_string_pretty(&tools_json).unwrap_or_default()
    )
}

/// Parse `<tool_call>...</tool_call>` blocks from model text output.
/// Returns `None` if no valid tool calls found.
pub(crate) fn parse_tool_calls_from_text(text: &str) -> Option<Vec<crate::ai::AiToolCall>> {
    let mut calls = Vec::new();
    let mut pos = 0;
    let open = "<tool_call>";
    let close = "</tool_call>";
    while let Some(start_offset) = text[pos..].find(open) {
        let content_start = pos + start_offset + open.len();
        if let Some(end_offset) = text[content_start..].find(close) {
            let json_str = &text[content_start..content_start + end_offset];
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                let name = val["name"].as_str().unwrap_or("").to_string();
                let args = val["arguments"].clone();
                if !name.is_empty() {
                    calls.push(crate::ai::AiToolCall {
                        id: format!("call_sp_{}", calls.len()),
                        tool_name: name,
                        args,
                    });
                }
                pos = content_start + end_offset + close.len();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    if calls.is_empty() { None } else { Some(calls) }
}
```

- [ ] **Step 2: 確認 helper 函數可加入 unit test**

在同一檔案的 `#[cfg(test)]` 區塊加入（若無 test 區塊則新建）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_calls_finds_single_call() {
        let text = r#"Some text before <tool_call>{"name":"brave__search","arguments":{"query":"WWDC 2026"}}</tool_call> after"#;
        let calls = parse_tool_calls_from_text(text).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "brave__search");
        assert_eq!(calls[0].args["query"], "WWDC 2026");
    }

    #[test]
    fn parse_tool_calls_returns_none_when_absent() {
        let text = "Just a plain answer, no tool calls here.";
        assert!(parse_tool_calls_from_text(text).is_none());
    }

    #[test]
    fn parse_tool_calls_finds_multiple_calls() {
        let text = r#"<tool_call>{"name":"tool_a","arguments":{"x":1}}</tool_call> and <tool_call>{"name":"tool_b","arguments":{}}</tool_call>"#;
        let calls = parse_tool_calls_from_text(text).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool_name, "tool_a");
        assert_eq!(calls[1].tool_name, "tool_b");
    }
}
```

- [ ] **Step 3: 執行 unit tests**

```bash
cd src-tauri && cargo test parse_tool_calls 2>&1 | tail -10
```

Expected:
```
test commands::ai::tests::parse_tool_calls_finds_single_call ... ok
test commands::ai::tests::parse_tool_calls_returns_none_when_absent ... ok
test commands::ai::tests::parse_tool_calls_finds_multiple_calls ... ok
```

- [ ] **Step 4: 修改 `ai_chat` 的 Unsupported 分支，加入 fallback 邏輯**

在 `ai_chat` 函數內，找到這段（約第 330 行）：

```rust
if !tools.is_empty() {
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_clone = provider.clone();
    let req_clone = req.clone();
    let join = tokio::spawn(async move {
        provider_clone.generate_with_tools(req_clone, tools, tx).await
    });
```

把 `tools` 在 spawn 前先 clone，並修改 `Unsupported` match arm：

```rust
if !tools.is_empty() {
    let tools_for_fallback = tools.clone(); // clone before moving into spawn
    let (tx, mut rx) = mpsc::channel::<GenerateChunk>(16);
    let provider_clone = provider.clone();
    let req_clone = req.clone();
    let join = tokio::spawn(async move {
        provider_clone.generate_with_tools(req_clone, tools, tx).await
    });

    while let Some(chunk) = rx.recv().await {
        let _ = app.emit("ai-stream", AiStreamEvent {
            session_id: session_id.clone(),
            kind: AiStreamKind::Chat,
            delta: chunk.delta.clone(),
            done: chunk.done,
        });
        if chunk.done { break; }
    }

    return match join.await {
        Ok(Ok(crate::ai::GenerateWithToolsResult::ToolCalls(calls))) =>
            Ok(AiChatReply { content: None, tool_calls: calls, tool_calling_unsupported: false }),
        Ok(Ok(crate::ai::GenerateWithToolsResult::Text(content))) =>
            Ok(AiChatReply { content: Some(content), tool_calls: vec![], tool_calling_unsupported: false }),
        Ok(Ok(crate::ai::GenerateWithToolsResult::Unsupported)) |
        Ok(Err(AiError::ToolCallingUnsupported)) => {
            // System prompt fallback: inject tool descriptions and re-call generate()
            let tool_injection = build_tool_prompt_injection(&tools_for_fallback);
            let mut fallback_req = req.clone();
            fallback_req.system_prompt =
                format!("{}\n\n{}", fallback_req.system_prompt, tool_injection);

            let (tx2, mut rx2) = mpsc::channel::<GenerateChunk>(16);
            let provider2 = provider.clone();
            let join2 = tokio::spawn(async move {
                provider2.generate(fallback_req, tx2).await
            });

            let mut buf2 = String::new();
            while let Some(chunk) = rx2.recv().await {
                let _ = app.emit("ai-stream", AiStreamEvent {
                    session_id: session_id.clone(),
                    kind: AiStreamKind::Chat,
                    delta: chunk.delta.clone(),
                    done: chunk.done,
                });
                buf2.push_str(&chunk.delta);
                if chunk.done { break; }
            }
            let _ = join2.await;

            if let Some(calls) = parse_tool_calls_from_text(&buf2) {
                Ok(AiChatReply { content: None, tool_calls: calls, tool_calling_unsupported: false })
            } else {
                Ok(AiChatReply { content: Some(buf2), tool_calls: vec![], tool_calling_unsupported: false })
            }
        }
        Ok(Err(e)) => Err(e),
        Err(e) => Err(AiError::Network { message: e.to_string() }),
    };
}
```

- [ ] **Step 5: 編譯確認**

```bash
cd src-tauri && cargo build 2>&1 | grep -E "^error"
```

Expected: 無輸出

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/ai.rs
git commit -m "feat(ai): system prompt fallback for MCP tool calling on unsupported providers"
```

---

## Task 3: useMcpChat — session history + complete interface

**Files:**
- Modify: `src/hooks/useMcpChat.ts`

### 背景
`AiPanel` 要切換到 `useMcpChat`，但目前它缺少 `useAiChat` 提供的介面：`sessions`、`loadMessages`、`deleteSession`、`addMessage`、`resend`、`error`、`clear`（alias）、`isStreaming`（alias）。

- [ ] **Step 1: 將整個 `useMcpChat.ts` 替換為以下完整版本**

```typescript
// src/hooks/useMcpChat.ts
import { useState, useCallback, useRef, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { aiChat, type AiToolCall } from "../ipc/ai";
import { executeMcpTool } from "../ipc/mcp";
import type { ChatMessage } from "../ipc/ai";

const MAX_TOOL_ITERATIONS = 10;
const SESSIONS_STORAGE_KEY = "aiterm-mcp-chat-sessions";

export interface McpChatMessage {
  role: "user" | "assistant" | "tool_call" | "tool_result";
  content: string;
  tool_name?: string;
  tool_call_id?: string;
  is_error?: boolean;
  is_loading?: boolean;
}

export interface McpChatSession {
  id: string;
  title: string;
  messages: McpChatMessage[];
  savedAt: number;
}

function loadAllSessions(): McpChatSession[] {
  try {
    const raw = localStorage.getItem(SESSIONS_STORAGE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveAllSessions(sessions: McpChatSession[]): void {
  try {
    localStorage.setItem(SESSIONS_STORAGE_KEY, JSON.stringify(sessions));
  } catch { /* ignore */ }
}

function formatSessionTitle(messages: McpChatMessage[]): string {
  const first = messages.find((m) => m.role === "user");
  return first ? first.content.slice(0, 30) : "（空對話）";
}

export function useMcpChat(sessionId: string) {
  const [messages, setMessages] = useState<McpChatMessage[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [streamBuf, setStreamBuf] = useState("");
  const [toolCallingUnsupported, setToolCallingUnsupported] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sessions, setSessions] = useState<McpChatSession[]>(loadAllSessions);
  const mountedRef = useRef(true);
  const lastSendRef = useRef<{ text: string; useMcp: boolean } | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  // Listen for streaming deltas from ai-stream events
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ session_id: string; kind: string; delta: string; done: boolean }>(
      "ai-stream",
      (event) => {
        if (event.payload.session_id !== sessionId) return;
        if (!mountedRef.current) return;
        if (event.payload.done) {
          setStreamBuf("");
        } else {
          setStreamBuf(prev => prev + event.payload.delta);
        }
      }
    ).then(u => { unlisten = u; });
    return () => { unlisten?.(); };
  }, [sessionId]);

  const saveSession = useCallback((msgs: McpChatMessage[]) => {
    if (msgs.length === 0) return;
    const session: McpChatSession = {
      id: `${Date.now()}`,
      title: formatSessionTitle(msgs),
      messages: msgs,
      savedAt: Date.now(),
    };
    setSessions(prev => {
      const updated = [session, ...prev].slice(0, 50);
      saveAllSessions(updated);
      return updated;
    });
  }, []);

  const sendMessage = useCallback(async (
    text: string,
    useMcp: boolean,
  ) => {
    if (!text.trim()) return;
    lastSendRef.current = { text, useMcp };

    setMessages(prev => [...prev, { role: "user", content: text }]);
    setIsLoading(true);
    setToolCallingUnsupported(false);
    setError(null);

    // Build the message history for the AI (user/assistant only)
    const historySnapshot = messages
      .filter(m => m.role === "user" || m.role === "assistant")
      .map(m => ({ role: m.role as "user" | "assistant", content: m.content }));

    const history: ChatMessage[] = [
      ...historySnapshot,
      { role: "user", content: text },
    ];

    try {
      let iterHistory = [...history];
      let iterations = 0;

      while (iterations < MAX_TOOL_ITERATIONS) {
        iterations++;
        const reply = await aiChat(iterHistory, sessionId, undefined, useMcp);

        if (!mountedRef.current) break;

        // System prompt fallback already handled in backend — no need for Unsupported special case
        if (reply.tool_calling_unsupported) {
          setToolCallingUnsupported(true);
          const fallback = await aiChat(iterHistory, sessionId, undefined, false);
          if (mountedRef.current) {
            setMessages(prev => {
              const updated = [...prev, { role: "assistant" as const, content: fallback.content ?? "" }];
              saveSession(updated);
              return updated;
            });
          }
          break;
        }

        // Handle tool calls
        if (reply.tool_calls.length > 0) {
          for (const tc of reply.tool_calls) {
            if (!mountedRef.current) break;
            setMessages(prev => [...prev, {
              role: "tool_call" as const,
              content: JSON.stringify(tc.args, null, 2),
              tool_name: tc.tool_name,
              tool_call_id: tc.id,
              is_loading: true,
            }]);
          }

          const toolResults: ChatMessage[] = [];
          for (const tc of reply.tool_calls) {
            let resultContent: string;
            let isError = false;
            try {
              const result = await executeMcpTool(tc.tool_name, tc.args);
              resultContent = result.content;
              isError = result.is_error;
            } catch (e) {
              resultContent = `Error: ${e}`;
              isError = true;
            }

            if (!mountedRef.current) break;

            setMessages(prev => prev.map(m =>
              m.tool_call_id === tc.id ? { ...m, is_loading: false, is_error: isError } : m
            ));
            setMessages(prev => [...prev, {
              role: "tool_result" as const,
              content: resultContent,
              tool_name: tc.tool_name,
              tool_call_id: tc.id,
              is_error: isError,
            }]);

            toolResults.push({
              role: "tool",
              content: resultContent,
            } as unknown as ChatMessage);
          }

          iterHistory = [
            ...iterHistory,
            { role: "assistant", content: encodeToolCalls(reply.tool_calls) } as unknown as ChatMessage,
            ...toolResults,
          ];
          continue;
        }

        // Normal text response — done
        setMessages(prev => {
          const updated = [...prev, { role: "assistant" as const, content: reply.content ?? streamBuf }];
          saveSession(updated);
          return updated;
        });
        break;
      }

      if (iterations >= MAX_TOOL_ITERATIONS && mountedRef.current) {
        setMessages(prev => [...prev, {
          role: "assistant" as const,
          content: "⚠️ 已達工具呼叫上限（10 次），請重新提問。",
        }]);
      }
    } catch (e) {
      if (mountedRef.current) {
        setError(String(e));
      }
    } finally {
      if (mountedRef.current) {
        setIsLoading(false);
        setStreamBuf("");
      }
    }
  }, [messages, sessionId, streamBuf, saveSession]);

  const addMessage = useCallback((msg: McpChatMessage) => {
    setMessages(prev => [...prev, msg]);
  }, []);

  const clearMessages = useCallback(() => {
    setMessages([]);
    setToolCallingUnsupported(false);
    setError(null);
  }, []);

  const loadMessages = useCallback((msgs: McpChatMessage[]) => {
    setMessages(msgs);
    setError(null);
  }, []);

  const deleteSession = useCallback((id: string) => {
    setSessions(prev => {
      const updated = prev.filter(s => s.id !== id);
      saveAllSessions(updated);
      return updated;
    });
  }, []);

  const resend = useCallback(async () => {
    if (!lastSendRef.current) return;
    // Remove messages from the last user message onwards and re-send
    setMessages(prev => {
      const lastUserIdx = [...prev].reverse().findIndex(m => m.role === "user");
      if (lastUserIdx < 0) return prev;
      return prev.slice(0, prev.length - lastUserIdx - 1);
    });
    const { text, useMcp } = lastSendRef.current;
    await sendMessage(text, useMcp);
  }, [sendMessage]);

  return {
    messages,
    isLoading,
    isStreaming: isLoading,   // alias for AiPanel compatibility
    streamBuf,
    error,
    toolCallingUnsupported,
    sendMessage,
    send: sendMessage,        // alias for AiPanel compatibility
    addMessage,
    clearMessages,
    clear: clearMessages,     // alias
    loadMessages,
    deleteSession,
    resend,
    sessions,
  };
}

function encodeToolCalls(tool_calls: AiToolCall[]): string {
  return JSON.stringify(tool_calls.map(tc => ({
    id: tc.id,
    type: "function",
    function: { name: tc.tool_name, arguments: JSON.stringify(tc.args) },
  })));
}
```

- [ ] **Step 2: 型別檢查**

```bash
cd /path/to/AITERM && npx tsc --noEmit 2>&1 | grep "useMcpChat\|McpChat" | head -20
```

Expected: 無錯誤輸出（若有錯誤先解決）

- [ ] **Step 3: 執行 frontend tests**

```bash
npm run test -- --run 2>&1 | tail -20
```

Expected: all tests pass (existing tests should still pass)

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useMcpChat.ts
git commit -m "feat(useMcpChat): add session history, addMessage, resend, error, compat aliases"
```

---

## Task 4: MessageList — tool call cards

**Files:**
- Modify: `src/components/AiPanel/MessageList.tsx`
- Modify: `src/components/AiPanel/styles.css`

### 背景
`MessageList` 目前只處理 `ChatMessage[]`。需要改成接受 `McpChatMessage[]`，並對 `tool_call` 和 `tool_result` 渲染可展開的工具卡片。`tool_result` 合併進對應的 `tool_call` 卡片（以 `tool_call_id` 配對）。

- [ ] **Step 1: 將 `MessageList.tsx` 替換為以下版本**

```tsx
// src/components/AiPanel/MessageList.tsx
import { useEffect, useRef, useState } from "react";
import type { AiError } from "../../ipc/ai";
import { formatAiError } from "../../ipc/ai";
import type { McpChatMessage } from "../../hooks/useMcpChat";
import { MessageBubble } from "./MessageBubble";

interface MessageListProps {
  messages: McpChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  error: AiError | string | null;
  onExecuteCommand: (cmd: string) => void;
  onRetry: () => void;
}

function formatError(error: AiError | string | null): string {
  if (!error) return "";
  if (typeof error === "string") return error;
  return formatAiError(error);
}

function ToolCallCard({
  callMsg,
  resultMsg,
}: {
  callMsg: McpChatMessage;
  resultMsg: McpChatMessage | undefined;
}) {
  const [expanded, setExpanded] = useState(false);
  const toolDisplayName = callMsg.tool_name?.includes("__")
    ? callMsg.tool_name.split("__").slice(1).join("__")
    : (callMsg.tool_name ?? "tool");

  const isLoading = callMsg.is_loading;
  const isError = resultMsg?.is_error ?? callMsg.is_error;
  const hasResult = !!resultMsg && !isLoading;

  return (
    <div className={`aiterm-tool-card${isError ? " aiterm-tool-card--error" : ""}`}>
      <button
        type="button"
        className="aiterm-tool-card-header"
        onClick={() => setExpanded(e => !e)}
      >
        <span className="aiterm-tool-card-icon">⚙</span>
        <span className="aiterm-tool-card-name">{toolDisplayName}</span>
        <span className="aiterm-tool-card-status">
          {isLoading && <span className="aiterm-tool-spinner">⟳</span>}
          {!isLoading && hasResult && !isError && "✓"}
          {!isLoading && isError && "✗"}
        </span>
        <span className="aiterm-tool-card-chevron">{expanded ? "▲" : "▼"}</span>
      </button>
      {expanded && (
        <div className="aiterm-tool-card-body">
          <div className="aiterm-tool-card-section">
            <div className="aiterm-tool-card-label">輸入</div>
            <pre className="aiterm-tool-card-content">{callMsg.content}</pre>
          </div>
          {hasResult && (
            <div className="aiterm-tool-card-section">
              <div className="aiterm-tool-card-label">輸出</div>
              <pre className="aiterm-tool-card-content">{resultMsg!.content}</pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

export function MessageList({
  messages,
  streamBuf,
  isStreaming,
  error,
  onExecuteCommand,
  onRetry,
}: MessageListProps) {
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, streamBuf, error]);

  // Build a map of tool_call_id → tool_result message for card merging
  const resultMap = new Map<string, McpChatMessage>();
  for (const m of messages) {
    if (m.role === "tool_result" && m.tool_call_id) {
      resultMap.set(m.tool_call_id, m);
    }
  }

  return (
    <div ref={listRef} className="aiterm-message-list">
      {messages.map((m, i) => {
        if (m.role === "tool_result") {
          // Rendered as part of the tool_call card above — skip
          return null;
        }
        if (m.role === "tool_call") {
          const result = m.tool_call_id ? resultMap.get(m.tool_call_id) : undefined;
          return (
            <ToolCallCard key={i} callMsg={m} resultMsg={result} />
          );
        }
        return (
          <MessageBubble
            key={i}
            role={m.role === "assistant" ? "assistant" : "user"}
            content={m.content}
            onExecuteCommand={onExecuteCommand}
          />
        );
      })}
      {isStreaming && streamBuf && (
        <MessageBubble
          role="assistant"
          content={streamBuf}
          onExecuteCommand={onExecuteCommand}
          streaming
        />
      )}
      {error && (
        <div className="aiterm-bubble aiterm-bubble-error" role="alert">
          <span>⚠ {formatError(error)}</span>
          <button
            type="button"
            className="aiterm-retry-btn"
            onClick={onRetry}
            disabled={isStreaming}
          >
            🔄 重試
          </button>
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: 在 `styles.css` 末尾加入工具卡片樣式**

```css
/* ── Tool call cards ── */
.aiterm-tool-card {
  margin: 4px 12px;
  border: 1px solid #2a2a2a;
  border-radius: 6px;
  overflow: hidden;
  font-size: 12px;
}

.aiterm-tool-card--error {
  border-color: #7c3a0a;
}

.aiterm-tool-card-header {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 5px 10px;
  background: #1a1a1a;
  border: none;
  cursor: pointer;
  color: #999;
  text-align: left;
}

.aiterm-tool-card-header:hover {
  background: #202020;
}

.aiterm-tool-card-name {
  flex: 1;
  color: #ccc;
  font-family: monospace;
}

.aiterm-tool-card-status {
  color: #34d399;
  min-width: 14px;
  text-align: center;
}

.aiterm-tool-card--error .aiterm-tool-card-status {
  color: #f87171;
}

.aiterm-tool-spinner {
  display: inline-block;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.aiterm-tool-card-chevron {
  font-size: 10px;
  color: #555;
}

.aiterm-tool-card-body {
  border-top: 1px solid #2a2a2a;
  background: #111;
}

.aiterm-tool-card-section {
  padding: 6px 10px;
  border-bottom: 1px solid #1e1e1e;
}

.aiterm-tool-card-section:last-child {
  border-bottom: none;
}

.aiterm-tool-card-label {
  font-size: 10px;
  color: #555;
  text-transform: uppercase;
  letter-spacing: 0.05em;
  margin-bottom: 3px;
}

.aiterm-tool-card-content {
  margin: 0;
  font-family: monospace;
  font-size: 11px;
  color: #aaa;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 200px;
  overflow-y: auto;
}
```

- [ ] **Step 3: 型別檢查**

```bash
npx tsc --noEmit 2>&1 | grep "MessageList" | head -10
```

Expected: 無錯誤

- [ ] **Step 4: 執行現有測試確認沒有 regression**

```bash
npm run test -- --run 2>&1 | tail -10
```

Expected: all tests pass

- [ ] **Step 5: Commit**

```bash
git add src/components/AiPanel/MessageList.tsx src/components/AiPanel/styles.css
git commit -m "feat(MessageList): add tool call cards for MCP tool execution display"
```

---

## Task 5: AiPanel — 切換到 useMcpChat

**Files:**
- Modify: `src/components/AiPanel/index.tsx`

### 背景
將 `AiPanel` 從 `useAiChat` 切換到 `useMcpChat`。`useMcpChat` 已有 compat aliases（`send`、`clear`、`isStreaming`），所以大部分呼叫不需修改。主要需要：更新 import、更新 `chat.messages` 型別宣告、更新傳給 `MessageList` 的 props（`messages` 現在是 `McpChatMessage[]`）。

- [ ] **Step 1: 更新 import**

在 `index.tsx` 頂端找到：

```typescript
import { useAiChat } from "../../hooks/useAiChat";
import { invokeAiChat, type ContentPart, type ChatMessage as AiChatMessage } from "../../ipc/ai";
```

替換為：

```typescript
import { useMcpChat, type McpChatMessage } from "../../hooks/useMcpChat";
import { invokeAiChat, type ContentPart, type ChatMessage as AiChatMessage } from "../../ipc/ai";
```

- [ ] **Step 2: 更新 hook 實例化**

找到：
```typescript
const chat = useAiChat(sessionId);
```

替換為：
```typescript
const chat = useMcpChat(sessionId);
```

- [ ] **Step 3: 更新 session history 的型別**

找到 `chat.loadMessages(s.messages)` 調用處（歷史面板）。`useMcpChat.sessions` 的 `messages` 型別是 `McpChatMessage[]`，`loadMessages` 接受 `McpChatMessage[]`，所以此行不需改動。

- [ ] **Step 4: 更新 `sendRemoteResponse` 的 messages 讀取**

找到這段（約第 252 行）：

```typescript
const lastMsg = chat.messages[chat.messages.length - 1];
if (lastMsg.role === "assistant" && sendRemoteResponse && !chat.isStreaming) {
  const text = typeof lastMsg.content === "string"
    ? lastMsg.content
    : (lastMsg.content as ContentPart[])
        .filter((p): p is Extract<ContentPart, { type: "text" }> => p.type === "text")
        .map((p) => p.text).join(" ");
  sendRemoteResponse(text);
}
```

替換為（`McpChatMessage.content` 永遠是 string）：

```typescript
const lastMsg = chat.messages[chat.messages.length - 1];
if (lastMsg?.role === "assistant" && sendRemoteResponse && !chat.isStreaming) {
  sendRemoteResponse(lastMsg.content);
}
```

- [ ] **Step 5: 更新 agentLoop 裡的 `chat.addMessage` 呼叫**

找到 agent loop 裡的 `chat.addMessage` 呼叫，確認其型別相容。目前呼叫為：

```typescript
chat.addMessage({ role: "assistant", content: reply });
```

`McpChatMessage` 接受 `role: "user" | "assistant" | "tool_call" | "tool_result"`，所以 `"assistant"` 相容，不需修改。

- [ ] **Step 6: 型別檢查整個 AiPanel**

```bash
npx tsc --noEmit 2>&1 | grep "AiPanel\|index.tsx" | head -20
```

Expected: 無錯誤。若有型別錯誤，根據錯誤訊息修正（通常是 `content` 型別差異）。

- [ ] **Step 7: 執行所有 frontend tests**

```bash
npm run test -- --run 2>&1 | tail -20
```

Expected: all tests pass

- [ ] **Step 8: 執行所有 Rust tests**

```bash
cd src-tauri && cargo test 2>&1 | tail -15
```

Expected: 無 FAILED

- [ ] **Step 9: Commit**

```bash
git add src/components/AiPanel/index.tsx
git commit -m "feat(AiPanel): switch to useMcpChat for MCP tool calling support"
```

---

## 驗收測試

啟動 app（`npm run tauri:dev`）後：

1. 在設定 → MCP Servers 確認至少一個 server 已連線（如 brave-search-mcp）
2. 開啟 AI Chat，確認底部顯示 `● MCP (N)` 且為綠色
3. 輸入：「本屆 WWDC 2026 宣布了哪些東西？請搜尋最新資訊」
4. 預期行為：
   - 出現工具卡片「⚙ search ⟳」（執行中）
   - 卡片更新為「⚙ search ✓」
   - AI 根據工具結果回答
5. 點擊工具卡片確認可展開 input/output
6. 測試非 Ollama provider（如 Anthropic）確認同樣運作
