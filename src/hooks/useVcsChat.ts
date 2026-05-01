import { useCallback, useEffect, useRef, useState } from "react";
import { vcsQuery, type VcsResult, type VcsRepoInfo } from "../ipc/vcs";

export interface VcsChatMessage {
  id: string;
  role: "user" | "assistant";
  text: string;
  result?: VcsResult;
}

export interface UseVcsChatResult {
  messages: VcsChatMessage[];
  isLoading: boolean;
  error: string | null;
  send: (text: string) => Promise<void>;
  clear: () => void;
}

export function useVcsChat(sessionId: string, repoInfo: VcsRepoInfo | null): UseVcsChatResult {
  const [messages, setMessages] = useState<VcsChatMessage[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // Reset messages when the repo root changes
  const prevRootRef = useRef<string | null>(null);
  useEffect(() => {
    const root = repoInfo?.root ?? null;
    if (root !== prevRootRef.current) {
      prevRootRef.current = root;
      setMessages([]);
      setError(null);
    }
  }, [repoInfo?.root]);

  const send = useCallback(async (text: string) => {
    if (isLoading || !repoInfo) return;

    const userMsg: VcsChatMessage = {
      id: crypto.randomUUID(),
      role: "user",
      text,
    };
    setMessages((prev) => [...prev, userMsg]);
    setIsLoading(true);
    setError(null);

    try {
      const result = await vcsQuery(text, repoInfo, sessionId);
      if (!mountedRef.current) return;

      // Derive display text from result
      let displayText = "";
      if (result.type === "error") {
        displayText = result.message;
      } else if (result.type === "write_success") {
        displayText = result.detail;
      } else if (result.type === "svn_not_installed") {
        displayText = "svn_not_installed";
      } else if (result.type === "no_token") {
        displayText = "no_token";
      } else {
        displayText = result.type;
      }

      const assistantMsg: VcsChatMessage = {
        id: crypto.randomUUID(),
        role: "assistant",
        text: displayText,
        result,
      };
      setMessages((prev) => [...prev, assistantMsg]);
    } catch (e) {
      if (!mountedRef.current) return;
      setError(String(e));
    } finally {
      if (mountedRef.current) {
        setIsLoading(false);
      }
    }
  }, [isLoading, repoInfo, sessionId]);

  const clear = useCallback(() => {
    setMessages([]);
    setError(null);
  }, []);

  return { messages, isLoading, error, send, clear };
}
