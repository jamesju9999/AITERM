import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { formatAiError, type AiError, type ChatMessage } from "../ipc/ai";
import {
  KB_CHAT_EVENT, invokeKbChat, type KbChatEvent,
  kbCreateChatSession, kbListChatSessions, kbLoadChatSession, kbDeleteChatSession,
  type ChatSessionSummary, type ChatMessageRow,
} from "../ipc/knowledgeBase";
import { useLocale } from "../contexts/LocaleContext";
import type { ToolCallState } from "./useCodeAssistant";

export interface KbMessage {
  role: "user" | "assistant";
  content: string;
  toolCalls?: ToolCallState[];
  checkpoints?: number[];
  streaming?: boolean;
}

const TITLE_MAX_LEN = 30;

function truncateTitle(text: string): string {
  const trimmed = text.trim();
  return trimmed.length > TITLE_MAX_LEN ? `${trimmed.slice(0, TITLE_MAX_LEN)}…` : trimmed;
}

interface PersistedToolCall {
  tool: string;
  args: Record<string, unknown>;
  result: string;
}

export function reconstructKbMessages(rows: ChatMessageRow[]): KbMessage[] {
  return rows.map((r, i) => {
    let toolCalls: ToolCallState[] | undefined;
    if (r.tool_calls_json) {
      try {
        const parsed = JSON.parse(r.tool_calls_json) as PersistedToolCall[];
        toolCalls = parsed.map((tc, j) => ({
          callId: `restored-${i}-${j}`,
          tool: tc.tool,
          args: tc.args,
          result: { content: tc.result, truncated: false },
        }));
      } catch {
        toolCalls = undefined;
      }
    }
    return {
      role: r.role === "user" ? "user" : "assistant",
      content: r.content,
      toolCalls,
    };
  });
}

export interface UseKnowledgeBaseChatResult {
  messages: KbMessage[];
  isStreaming: boolean;
  error: string | null;
  isFallbackMode: boolean;
  tokenCount: number;
  tokenLimit: number;
  sessions: ChatSessionSummary[];
  activeChatSessionId: string | null;
  send: (userText: string, providerId?: string) => Promise<void>;
  clear: () => void;
  loadSession: (sessionId: string) => Promise<void>;
  deleteSession: (sessionId: string) => Promise<void>;
}

export function useKnowledgeBaseChat(notebookId: string | null): UseKnowledgeBaseChatResult {
  const [messages, setMessages] = useState<KbMessage[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isFallbackMode, setIsFallbackMode] = useState(false);
  const [tokenCount, setTokenCount] = useState(0);
  const [tokenLimit, setTokenLimit] = useState(50000);
  const [sessions, setSessions] = useState<ChatSessionSummary[]>([]);
  const [activeChatSessionId, setActiveChatSessionId] = useState<string | null>(null);
  const mountedRef = useRef(true);
  const notebookIdRef = useRef(notebookId);
  const { locale } = useLocale();

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  useEffect(() => {
    notebookIdRef.current = notebookId;
  }, [notebookId]);

  const refreshSessions = useCallback(async (nbId: string) => {
    try {
      const list = await kbListChatSessions(nbId);
      if (mountedRef.current) setSessions(list);
    } catch { /* ignore */ }
  }, []);

  // 切換筆記本時重置對話狀態，避免把上一個筆記本的對話帶到新的筆記本；
  // 重新載入該筆記本自己的對話記錄清單，但不自動接續舊對話。
  useEffect(() => {
    setMessages([]);
    setError(null);
    setIsFallbackMode(false);
    setTokenCount(0);
    setActiveChatSessionId(null);
    setSessions([]);
    if (notebookId) void refreshSessions(notebookId);
  }, [notebookId, refreshSessions]);

  const send = useCallback(async (userText: string, providerId?: string) => {
    if (!userText.trim() || isStreaming || !notebookId) return;
    setError(null);
    setIsStreaming(true);

    let chatSessionId = activeChatSessionId;
    if (!chatSessionId) {
      try {
        chatSessionId = await kbCreateChatSession(notebookId, truncateTitle(userText));
      } catch (e) {
        setError(String(e));
        setIsStreaming(false);
        return;
      }
      if (notebookIdRef.current !== notebookId) {
        // notebook changed while this session was being created — abandon this send,
        // the session row was created but nothing in current UI state should reference it
        setIsStreaming(false);
        return;
      }
      setActiveChatSessionId(chatSessionId);
      void refreshSessions(notebookId);
    }

    const chatMessages: ChatMessage[] = [
      ...messages.map((m) => ({ role: m.role, content: m.content })),
      { role: "user" as const, content: userText },
    ];

    setMessages((prev) => [
      ...prev,
      { role: "user", content: userText },
      { role: "assistant", content: "", toolCalls: [], streaming: true },
    ]);

    const sessionId = crypto.randomUUID();

    const unlisten = await listen<KbChatEvent>(KB_CHAT_EVENT, (event) => {
      if (!mountedRef.current) return;
      const p = event.payload;
      if (p.session_id !== sessionId) return;

      if (p.kind === "tool_call") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.toolCalls = [...(last.toolCalls ?? []), { callId: p.call_id, tool: p.tool, args: p.args }];
          next[next.length - 1] = last;
          return next;
        });
      } else if (p.kind === "tool_result") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.toolCalls = (last.toolCalls ?? []).map((tc) =>
            tc.callId === p.call_id ? { ...tc, result: { content: p.content, truncated: p.truncated } } : tc,
          );
          next[next.length - 1] = last;
          return next;
        });
      } else if (p.kind === "text_delta") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.content = (last.content ?? "") + p.delta;
          next[next.length - 1] = last;
          return next;
        });
      } else if (p.kind === "checkpoint") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.checkpoints = [...(last.checkpoints ?? []), p.number];
          next[next.length - 1] = last;
          return next;
        });
      } else if (p.kind === "fallback_mode") {
        setIsFallbackMode(true);
      } else if (p.kind === "token_count") {
        setTokenCount(p.count);
        setTokenLimit(p.limit);
      } else if (p.kind === "done") {
        setMessages((prev) => {
          const next = [...prev];
          next[next.length - 1] = { ...next[next.length - 1], streaming: false };
          return next;
        });
        setIsStreaming(false);
        unlisten();
        void refreshSessions(notebookId);
      } else if (p.kind === "error") {
        setError(p.message);
        setIsStreaming(false);
        setMessages((prev) => {
          const next = [...prev];
          next[next.length - 1] = { ...next[next.length - 1], streaming: false };
          return next;
        });
        unlisten();
      }
    });

    try {
      await invokeKbChat(notebookId, chatMessages, sessionId, chatSessionId, providerId, locale);
    } catch (e) {
      if (mountedRef.current) {
        const isAiError = e != null && typeof e === "object" && "kind" in (e as object);
        setError(isAiError ? formatAiError(e as AiError) : String(e));
        setIsStreaming(false);
        setMessages((prev) => {
          const next = [...prev];
          next[next.length - 1] = { ...next[next.length - 1], streaming: false };
          return next;
        });
        unlisten();
      }
    }
  }, [messages, isStreaming, locale, notebookId, activeChatSessionId, refreshSessions]);

  const clear = useCallback(() => {
    setMessages([]);
    setError(null);
    setIsFallbackMode(false);
    setTokenCount(0);
    setActiveChatSessionId(null);
  }, []);

  const loadSession = useCallback(async (sessionId: string) => {
    if (isStreaming) return;
    const rows = await kbLoadChatSession(sessionId);
    setMessages(reconstructKbMessages(rows));
    setActiveChatSessionId(sessionId);
    setError(null);
    setIsFallbackMode(false);
    setTokenCount(0);
  }, [isStreaming]);

  const deleteSession = useCallback(async (sessionId: string) => {
    await kbDeleteChatSession(sessionId);
    if (activeChatSessionId === sessionId) {
      setMessages([]);
      setActiveChatSessionId(null);
    }
    if (notebookId) void refreshSessions(notebookId);
  }, [activeChatSessionId, notebookId, refreshSessions]);

  return {
    messages, isStreaming, error, isFallbackMode, tokenCount, tokenLimit,
    sessions, activeChatSessionId, send, clear, loadSession, deleteSession,
  };
}
