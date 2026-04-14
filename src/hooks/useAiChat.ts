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

const HISTORY_LIMIT = 20;

export interface UseAiChatResult {
  messages: ChatMessage[];
  streamBuf: string;
  isStreaming: boolean;
  error: AiError | null;
  send: (userText: string) => Promise<void>;
  resend: () => Promise<void>;
  clear: () => void;
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
        fn();
      } else {
        unlisten = fn;
      }
    });
    return () => {
      active = false;
      if (unlisten) unlisten();
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
    async (userText: string) => {
      if (isStreaming) return; // UI should disable input anyway
      const userMsg: ChatMessage = { role: "user", content: userText };
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

  return { messages, streamBuf, isStreaming, error, send, resend, clear };
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
