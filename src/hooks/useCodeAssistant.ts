import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { formatAiError, type AiError, type ChatMessage } from "../ipc/ai";
import {
  CODE_ASSISTANT_EVENT,
  invokeCodeAssistantChat,
  type CodeAssistantEvent,
} from "../ipc/codeAssistant";
import { useLocale } from "../contexts/LocaleContext";

export interface ToolCallState {
  callId: string;
  tool: string;
  args: Record<string, unknown>;
  progress?: string;
  result?: { content: string; truncated: boolean };
}

export interface CodeMessage {
  role: "user" | "assistant";
  content: string;
  toolCalls?: ToolCallState[];
  checkpoints?: number[];
  streaming?: boolean;
}

export interface UseCodeAssistantResult {
  messages: CodeMessage[];
  isStreaming: boolean;
  error: string | null;
  isFallbackMode: boolean;
  tokenCount: number;
  tokenLimit: number;
  send: (userText: string, projectRoot: string, providerId?: string) => Promise<void>;
  clear: () => void;
}

export function useCodeAssistant(): UseCodeAssistantResult {
  const [messages, setMessages] = useState<CodeMessage[]>([]);
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [isFallbackMode, setIsFallbackMode] = useState(false);
  const [tokenCount, setTokenCount] = useState(0);
  const [tokenLimit, setTokenLimit] = useState(50000);
  const mountedRef = useRef(true);
  const { locale } = useLocale();

  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  const send = useCallback(async (
    userText: string,
    projectRoot: string,
    providerId?: string,
  ) => {
    if (!userText.trim() || isStreaming) return;
    setError(null);

    const chatMessages: ChatMessage[] = [
      ...messages.map((m) => ({ role: m.role, content: m.content })),
      { role: "user" as const, content: userText },
    ];

    setMessages((prev) => [
      ...prev,
      { role: "user", content: userText },
      { role: "assistant", content: "", toolCalls: [], streaming: true },
    ]);
    setIsStreaming(true);

    const sessionId = crypto.randomUUID();

    const unlisten = await listen<CodeAssistantEvent>(CODE_ASSISTANT_EVENT, (event) => {
      if (!mountedRef.current) return;
      const p = event.payload;
      if (p.session_id !== sessionId) return;

      if (p.kind === "tool_call") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.toolCalls = [
            ...(last.toolCalls ?? []),
            { callId: p.call_id, tool: p.tool, args: p.args },
          ];
          next[next.length - 1] = last;
          return next;
        });
      } else if (p.kind === "tool_progress") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.toolCalls = (last.toolCalls ?? []).map((tc) =>
            tc.callId === p.call_id ? { ...tc, progress: p.message } : tc,
          );
          next[next.length - 1] = last;
          return next;
        });
      } else if (p.kind === "tool_result") {
        setMessages((prev) => {
          const next = [...prev];
          const last = { ...next[next.length - 1] };
          last.toolCalls = (last.toolCalls ?? []).map((tc) =>
            tc.callId === p.call_id
              ? { ...tc, result: { content: p.content, truncated: p.truncated } }
              : tc,
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
      await invokeCodeAssistantChat(projectRoot, chatMessages, sessionId, providerId, locale);
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
  }, [messages, isStreaming, locale]);

  const clear = useCallback(() => {
    setMessages([]);
    setError(null);
    setIsFallbackMode(false);
    setTokenCount(0);
  }, []);

  return { messages, isStreaming, error, isFallbackMode, tokenCount, tokenLimit, send, clear };
}
