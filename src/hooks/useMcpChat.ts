// src/hooks/useMcpChat.ts
import { useState, useCallback, useRef, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { aiChat, formatAiError, type AiToolCall, type AiError } from "../ipc/ai";
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
  const streamBufRef = useRef("");
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
          streamBufRef.current = "";
        } else {
          setStreamBuf(prev => {
            streamBufRef.current = prev + event.payload.delta;
            return streamBufRef.current;
          });
        }
      }
    ).then(u => { unlisten = u; });
    return () => { unlisten?.(); };
  }, [sessionId]);

  const saveSession = useCallback((msgs: McpChatMessage[]) => {
    if (msgs.length === 0) return;
    setSessions(prev => {
      const existing = prev.findIndex(s => s.id === sessionId);
      const entry: McpChatSession = {
        id: sessionId,
        title: formatSessionTitle(msgs),
        messages: msgs,
        savedAt: Date.now(),
      };
      const updated = existing >= 0
        ? [entry, ...prev.filter(s => s.id !== sessionId)].slice(0, 50)
        : [entry, ...prev].slice(0, 50);
      saveAllSessions(updated);
      return updated;
    });
  }, [sessionId]);

  const sendMessage = useCallback(async (
    text: string,
    useMcp: boolean,
    existingMessages?: McpChatMessage[],
  ) => {
    if (!text.trim()) return;
    lastSendRef.current = { text, useMcp };

    setMessages(prev => [...prev, { role: "user", content: text }]);
    setIsLoading(true);
    setError(null);

    // Build the message history for the AI (user/assistant only)
    const baseMessages = existingMessages ?? messages;
    const historySnapshot = baseMessages
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

          // Execute all tools and collect results
          const executedResults: { tc: AiToolCall; result: string; isError: boolean }[] = [];
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

            executedResults.push({ tc, result: resultContent, isError });
          }

          // Build the next-turn history in a format all models understand:
          // assistant message uses <tool_call> tags (matches system prompt injection format),
          // tool results come back as a user message so the model can continue naturally.
          const assistantToolMsg: ChatMessage = {
            role: "assistant",
            content: reply.tool_calls
              .map(tc => `<tool_call>${JSON.stringify({ name: tc.tool_name, arguments: tc.args })}</tool_call>`)
              .join("\n"),
          };
          const toolResultMsg: ChatMessage = {
            role: "user",
            content: executedResults
              .map(({ tc, result }) => `Tool result for ${tc.tool_name}:\n${result}`)
              .join("\n\n"),
          };

          iterHistory = [...iterHistory, assistantToolMsg, toolResultMsg];
          continue;
        }

        // Normal text response — done
        setMessages(prev => {
          const updated = [...prev, { role: "assistant" as const, content: reply.content ?? streamBufRef.current }];
          saveSession(updated);
          return updated;
        });
        break;
      }

      if (iterations >= MAX_TOOL_ITERATIONS && mountedRef.current) {
        setMessages(prev => {
          const updated = [...prev, {
            role: "assistant" as const,
            content: "⚠️ 已達工具呼叫上限（10 次），請重新提問。",
          }];
          saveSession(updated);
          return updated;
        });
      }
    } catch (e) {
      if (mountedRef.current) {
        // e may be an AiError object from Tauri IPC — use formatAiError to
        // produce a readable message instead of "[object Object]"
        const isAiError = e != null && typeof e === "object" && "kind" in (e as object);
        setError(isAiError ? formatAiError(e as AiError) : String(e));
      }
    } finally {
      if (mountedRef.current) {
        setIsLoading(false);
        setStreamBuf("");
      }
    }
  }, [messages, sessionId, saveSession]);

  const addMessage = useCallback((msg: McpChatMessage) => {
    setMessages(prev => [...prev, msg]);
  }, []);

  const clearMessages = useCallback(() => {
    setMessages([]);
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
    if (!lastSendRef.current || isLoading) return;
    const { text, useMcp } = lastSendRef.current;
    const lastUserIdx = [...messages].reverse().findIndex(m => m.role === "user");
    const trimmedMessages = lastUserIdx >= 0
      ? messages.slice(0, messages.length - lastUserIdx - 1)
      : messages;
    setMessages(trimmedMessages);
    await sendMessage(text, useMcp, trimmedMessages);
  }, [messages, isLoading, sendMessage]);

  return {
    messages,
    isLoading,
    isStreaming: isLoading,   // alias for AiPanel compatibility
    streamBuf,
    error,
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

