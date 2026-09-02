// src/hooks/useRemoteAiChat.ts
import { useState, useCallback, useRef, useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { invokeAiChatCtx, formatAiError, type AiError, type RemoteCtx } from "../ipc/ai";
import type { ChatMessage } from "../ipc/ai";
import { useLocale } from "../contexts/LocaleContext";
import type { McpChatMessage, McpChatSession } from "../types/chat";

const SESSIONS_STORAGE_KEY = "aiterm-remote-chat-sessions";

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
  if (!first) return t.chat_empty;
  return typeof first.content === "string" ? first.content.slice(0, 30) : t.chat_empty;
}

export function useRemoteAiChat(connId: string, buildCtx: () => RemoteCtx, providerId?: string) {
  const { t, locale } = useLocale();
  const [messages, setMessages] = useState<McpChatMessage[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [streamBuf, setStreamBuf] = useState("");
  const streamBufRef = useRef("");
  const [error, setError] = useState<string | null>(null);
  const [sessions, setSessions] = useState<McpChatSession[]>(loadAllSessions);
  const mountedRef = useRef(true);
  const lastSendRef = useRef<{ text: string } | null>(null);
  const currentSessionIdRef = useRef<string | null>(null);

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
        if (event.payload.session_id !== connId) return;
        if (event.payload.kind !== "chat") return;
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
  }, [connId]);

  // Auto-save messages to localStorage whenever they change (non-empty only)
  useEffect(() => {
    if (messages.length === 0) return;
    if (!currentSessionIdRef.current) {
      currentSessionIdRef.current = `${connId}-${Date.now()}`;
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
  }, [messages, connId]);

  const sendMessage = useCallback(async (
    text: string,
    existingMessages?: McpChatMessage[],
  ) => {
    if (!text.trim()) return;
    lastSendRef.current = { text };

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
      const reply = await invokeAiChatCtx(history, buildCtx(), connId, providerId, locale, true);
      if (!mountedRef.current) return;
      setMessages(prev => [...prev, { role: "assistant", content: reply.content ?? streamBufRef.current }]);
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
  }, [messages, connId, providerId, locale, buildCtx]);

  const addMessage = useCallback((msg: McpChatMessage) => {
    setMessages(prev => [...prev, msg]);
  }, []);

  const clear = useCallback(() => {
    setMessages([]);
    setError(null);
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
    const { text } = lastSendRef.current;
    const lastUserIdx = [...messages].reverse().findIndex(m => m.role === "user");
    const trimmedMessages = lastUserIdx >= 0
      ? messages.slice(0, messages.length - lastUserIdx - 1)
      : messages;
    setMessages(trimmedMessages);
    await sendMessage(text, trimmedMessages);
  }, [messages, isLoading, sendMessage]);

  return {
    messages,
    isStreaming: isLoading,
    streamBuf,
    error,
    send: sendMessage,
    addMessage,
    clear,
    loadMessages,
    deleteSession,
    resend,
    sessions,
  };
}
