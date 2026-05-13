import { useCallback, useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import {
  invokeAiChat,
  type AiChatReply,
  type AiError,
  type AiStreamEvent,
  type ChatMessage,
} from "../ipc/ai";
import { truncateHistory } from "../lib/chatHistory";
import { buildContentParts, contentToDisplayString } from "../types/attachment";
import type { Attachment } from "../types/attachment";

const HISTORY_LIMIT = 20;
const SESSIONS_STORAGE_KEY = "aiterm-ai-chat-sessions";

export interface AiChatSession {
  id: string;
  title: string;
  messages: ChatMessage[];
  savedAt: number;
}

function loadAllSessions(): AiChatSession[] {
  try {
    const raw = localStorage.getItem(SESSIONS_STORAGE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveAllSessions(sessions: AiChatSession[]): void {
  try {
    localStorage.setItem(SESSIONS_STORAGE_KEY, JSON.stringify(sessions));
  } catch { /* ignore storage errors */ }
}

function formatSessionTitle(messages: ChatMessage[]): string {
  const first = messages.find((m) => m.role === "user");
  if (!first) return "（空對話）";
  const text = contentToDisplayString(first.content);
  return text.slice(0, 30) || "（附件）";
}

export interface UseAiChatResult {
  messages: ChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  error: AiError | null;
  send: (userText: string, attachments?: Attachment[]) => Promise<void>;
  resend: () => Promise<void>;
  clear: () => void;
  /** Inject a message directly into the chat history without calling the AI. */
  addMessage: (msg: ChatMessage) => void;
  /** Load a saved session's messages into the current chat. */
  loadMessages: (msgs: ChatMessage[]) => void;
  /** All persisted sessions for the history panel. */
  sessions: AiChatSession[];
  /** Delete a session by id. */
  deleteSession: (id: string) => void;
}

/**
 * Owns multi-turn chat state for the AI Panel. One instance per session:
 * remount (via `key={sessionId}`) resets all state.
 */
export function useAiChat(sessionId: string): UseAiChatResult {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [streamBuf, setStreamBuf] = useState("");
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<AiError | null>(null);
  const [sessions, setSessions] = useState<AiChatSession[]>(loadAllSessions);

  // Track current session id for auto-save
  const currentSessionIdRef = useRef<string | null>(null);

  // Guard against setState after unmount (Tauri listen + async invoke race).
  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Listen for streaming chunks.
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let active = true;
    listen<AiStreamEvent>("ai-stream", (event) => {
      if (!active) return;
      if (event.payload.kind !== "chat") return;
      if (event.payload.session_id !== sessionId) return;
      if (event.payload.done) return; // end-of-stream handled by invoke resolve
      setStreamBuf((prev) => prev + event.payload.delta);
    }).then((fn) => {
      if (!active) {
        // Cleanup ran before listen() resolved — unlisten and swallow
        // any rejection (Tauri internals throw if the event was never
        // fully wired up).
        Promise.resolve(fn()).catch(() => { /* ignore */ });
      } else {
        unlisten = fn;
      }
    }).catch((err) => {
      console.error("[ai-stream] listener registration failed:", err);
    });
    return () => {
      active = false;
      if (unlisten) {
        try {
          Promise.resolve(unlisten()).catch(() => { /* ignore */ });
        } catch { /* ignore */ }
      }
    };
  }, [sessionId]);

  const invokeChat = useCallback(
    async (msgs: ChatMessage[]) => {
      setStreamBuf("");
      setIsStreaming(true);
      setError(null);
      try {
        const reply: AiChatReply = await invokeAiChat(msgs, sessionId);
        if (!mountedRef.current) return;
        setMessages([...msgs, { role: "assistant", content: reply.content }]);
      } catch (e) {
        if (!mountedRef.current) return;
        setError(normalizeAiError(e));
        // Do NOT roll back `msgs` — user message stays so UI can show a retry.
      } finally {
        if (mountedRef.current) {
          setStreamBuf("");
          setIsStreaming(false);
        }
      }
    },
    [sessionId],
  );

  const send = useCallback(
    async (userText: string, attachments?: Attachment[]) => {
      if (isStreaming) return;
      const content =
        attachments && attachments.length > 0
          ? buildContentParts(userText, attachments)
          : userText;
      const userMsg: ChatMessage = { role: "user", content };
      const next = truncateHistory([...messages, userMsg], HISTORY_LIMIT);
      setMessages(next);
      await invokeChat(next);
    },
    [messages, isStreaming, invokeChat],
  );

  const resend = useCallback(async () => {
    if (isStreaming) return;
    if (messages.length === 0) return;
    if (messages[messages.length - 1].role !== "user") return;
    await invokeChat(messages);
  }, [messages, isStreaming, invokeChat]);

  const clear = useCallback(() => {
    if (isStreaming) return; // defence in depth; UI also disables button
    setMessages([]);
    setError(null);
    setStreamBuf("");
  }, [isStreaming]);

  const addMessage = useCallback((msg: ChatMessage) => {
    setMessages((prev) => [...prev, msg]);
  }, []);

  // Auto-save messages to localStorage whenever they change (non-empty only)
  useEffect(() => {
    if (messages.length === 0) return;
    const title = formatSessionTitle(messages);
    const all = loadAllSessions();
    if (!currentSessionIdRef.current) {
      currentSessionIdRef.current = `${sessionId}-${Date.now()}`;
    }
    const id = currentSessionIdRef.current;
    const updated: AiChatSession = { id, title, messages, savedAt: Date.now() };
    const idx = all.findIndex((s) => s.id === id);
    const next = idx >= 0
      ? all.map((s) => (s.id === id ? updated : s))
      : [...all, updated];
    saveAllSessions(next);
    setSessions(next);
  }, [messages, sessionId]);

  const loadMessages = useCallback((msgs: ChatMessage[]) => {
    setMessages(msgs);
    setError(null);
    setStreamBuf("");
    // Will be re-assigned on next auto-save; reset so a new session ID is minted
    currentSessionIdRef.current = null;
  }, []);

  const deleteSession = useCallback((id: string) => {
    setSessions((prev) => {
      const next = prev.filter((s) => s.id !== id);
      saveAllSessions(next);
      return next;
    });
    if (currentSessionIdRef.current === id) {
      currentSessionIdRef.current = null;
    }
  }, []);

  return { messages, streamBuf, isStreaming, error, send, resend, clear, addMessage, loadMessages, sessions, deleteSession };
}

/** Coerce an unknown Tauri error into an AiError. Mirrors TerminalView logic. */
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
