// src/hooks/useMcpChat.ts
import { useState, useCallback, useRef, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { unlistenOnCleanup } from "../lib/eventSubscription";
import { aiChat, formatAiError, type AiToolCall, type AiError, type ToolFallbackReason } from "../ipc/ai";
import { executeMcpTool } from "../ipc/mcp";
import type { ChatMessage } from "../ipc/ai";
import { buildContentParts, contentToDisplayString } from "../types/attachment";
import type { Attachment } from "../types/attachment";
import { useLocale } from "../contexts/LocaleContext";
import { intentNudge, looksLikeUnfulfilledIntent } from "../lib/unfulfilledIntent";
import type { McpChatMessage, McpChatSession } from "../types/chat";
export type { McpChatMessage, McpChatSession }; // re-export：既有 import { McpChatMessage } from "../hooks/useMcpChat" 不用改

// 呼叫端沒指定時的預設值；AiPanel 會傳入系統設定的「最大步數」（0 = 無限制，已換成大數）。
const DEFAULT_MAX_TOOL_ITERATIONS = 10;
// 模型只宣告動作卻沒呼叫工具時，最多催幾次。
const MAX_INTENT_NUDGES = 2;
const SESSIONS_STORAGE_KEY = "aiterm-mcp-chat-sessions";

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

function formatSessionTitle(messages: McpChatMessage[], t: any): string {
  const first = messages.find((m) => m.role === "user");
  return first ? contentToDisplayString(first.content).slice(0, 30) : t.chat_empty;
}

export function useMcpChat(sessionId: string, maxToolIterations: number = DEFAULT_MAX_TOOL_ITERATIONS) {
  const { t, locale } = useLocale();
  // 用 ref 讀最新值：sendMessage 是 useCallback，設定頁剛改完的上限不能被舊閉包吃掉。
  const maxIterationsRef = useRef(maxToolIterations);
  maxIterationsRef.current = maxToolIterations;
  const [messages, setMessages] = useState<McpChatMessage[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [streamBuf, setStreamBuf] = useState("");
  // 後端把工具呼叫切換成「注入系統提示」的文字協定時，這裡是切換的原因。
  // 用原因而不是布林：模型做不到、跟這張憑證的計費歸屬，是兩件該講不同話的事。
  const [toolFallbackReason, setToolFallbackReason] = useState<ToolFallbackReason | null>(null);
  const streamBufRef = useRef("");
  const [error, setError] = useState<string | null>(null);
  const [sessions, setSessions] = useState<McpChatSession[]>(loadAllSessions);
  const mountedRef = useRef(true);
  const lastSendRef = useRef<{ text: string; useMcp: boolean; attachments?: Attachment[] } | null>(null);
  const currentSessionIdRef = useRef<string | null>(null);

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  // Listen for streaming deltas from ai-stream events
  useEffect(() => {
    // cleanup 可能在 listen() resolve 之前就跑（StrictMode 雙重掛載、session 來回切換）：
    // 那時拿不到 unlisten，舊 listener 會永遠留著，之後每個串流 delta 就被處理兩次
    // （畫面上每個字都出現兩次）。所以 handler 自己檢查 active，並用 unlistenOnCleanup
    // 在 resolve 之後補解除——兩種順序都安全，也是冪等的。
    let active = true;
    const pending = listen<{ session_id: string; kind: string; delta: string; done: boolean }>(
      "ai-stream",
      (event) => {
        if (!active) return;
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
    );
    const unlisten = unlistenOnCleanup(pending, "ai-stream");
    return () => { active = false; unlisten(); };
  }, [sessionId]);

  // Auto-save messages to localStorage whenever they change (non-empty only)
  useEffect(() => {
    if (messages.length === 0) return;
    if (!currentSessionIdRef.current) {
      currentSessionIdRef.current = `${sessionId}-${Date.now()}`;
    }
    const id = currentSessionIdRef.current;
    const entry: McpChatSession = {
      id,
      title: formatSessionTitle(messages, t),
      messages,
      savedAt: Date.now(),
    };
    const all = loadAllSessions();
    const idx = all.findIndex(s => s.id === id);
    const next = (idx >= 0
      ? [entry, ...all.filter(s => s.id !== id)]
      : [entry, ...all]
    ).slice(0, 50);
    saveAllSessions(next);
    setSessions(next);
  }, [messages, sessionId]);

  const sendMessage = useCallback(async (
    text: string,
    useMcp: boolean,
    existingMessages?: McpChatMessage[],
    attachments?: Attachment[],
  ) => {
    if (!text.trim() && (!attachments || attachments.length === 0)) return;
    lastSendRef.current = { text, useMcp, attachments };

    const hasAttachments = attachments && attachments.length > 0;
    const userContent = hasAttachments ? buildContentParts(text, attachments) : text;
    setMessages(prev => [...prev, { role: "user", content: userContent }]);
    setIsLoading(true);
    setError(null);

    // Build the message history for the AI (user/assistant only)
    const baseMessages = existingMessages ?? messages;
    const historySnapshot = baseMessages
      .filter(m => m.role === "user" || m.role === "assistant")
      .map(m => ({ role: m.role as "user" | "assistant", content: m.content }));

    const history: ChatMessage[] = [
      ...historySnapshot,
      { role: "user", content: userContent },
    ];

    try {
      let iterHistory = [...history];
      let iterations = 0;
      let nudges = 0;

      const maxIterations = maxIterationsRef.current;
      while (iterations < maxIterations) {
        iterations++;
        const reply = await aiChat(iterHistory, sessionId, undefined, useMcp, locale, true);

        if (!mountedRef.current) break;

        if (reply.tool_calling_unsupported) {
          setToolFallbackReason(reply.tool_fallback_reason ?? "unsupported");
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

        const finalText = reply.content ?? streamBufRef.current;
        const isEmpty = finalText.trim() === "";

        // 模型沒呼叫工具就結束這一輪，有兩種常見情形，都讓任務停在半路：
        //  1. 只說「讓我先…」卻沒動手；2. 回了空內容（畫面上就是一顆空白氣泡）。
        // 催模型直接動手。催促只送給模型（iterHistory），不顯示在對話裡；有次數上限。
        if (useMcp && nudges < MAX_INTENT_NUDGES && (isEmpty || looksLikeUnfulfilledIntent(finalText))) {
          nudges++;
          const nudge = intentNudge(locale);
          if (isEmpty) {
            // 空回覆不進對話；也不能讓歷史出現連續兩則 user（上一則通常是工具結果），
            // 所以把催促併進上一則 user 訊息。
            const last = iterHistory[iterHistory.length - 1];
            iterHistory = last && last.role === "user" && typeof last.content === "string"
              ? [...iterHistory.slice(0, -1), { ...last, content: `${last.content}\n\n${nudge}` }]
              : [...iterHistory, { role: "user", content: nudge }];
          } else {
            setMessages(prev => [...prev, { role: "assistant" as const, content: finalText }]);
            iterHistory = [
              ...iterHistory,
              { role: "assistant", content: finalText },
              { role: "user", content: nudge },
            ];
          }
          continue;
        }

        // 催完還是空的：給明確的說明，而不是留一顆空白氣泡。
        setMessages(prev => [...prev, {
          role: "assistant" as const,
          content: isEmpty ? t.mcp_empty_reply : finalText,
        }]);
        break;
      }

      if (iterations >= maxIterations && mountedRef.current) {
        setMessages(prev => [...prev, {
          role: "assistant" as const,
          content: t.mcp_tool_limit.replace("{n}", String(maxIterations)),
        }]);
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
  }, [messages, sessionId, locale]);

  const addMessage = useCallback((msg: McpChatMessage) => {
    setMessages(prev => [...prev, msg]);
  }, []);

  const clearMessages = useCallback(() => {
    setMessages([]);
    setError(null);
    setToolFallbackReason(null);
    currentSessionIdRef.current = null;
  }, []);

  const loadMessages = useCallback((msgs: McpChatMessage[], loadedSessionId?: string) => {
    currentSessionIdRef.current = loadedSessionId ?? null;
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
    const { text, useMcp, attachments } = lastSendRef.current;
    const lastUserIdx = [...messages].reverse().findIndex(m => m.role === "user");
    const trimmedMessages = lastUserIdx >= 0
      ? messages.slice(0, messages.length - lastUserIdx - 1)
      : messages;
    setMessages(trimmedMessages);
    await sendMessage(text, useMcp, trimmedMessages, attachments);
  }, [messages, isLoading, sendMessage]);

  return {
    messages,
    isLoading,
    isStreaming: isLoading,   // alias for AiPanel compatibility
    streamBuf,
    toolFallbackReason,
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

