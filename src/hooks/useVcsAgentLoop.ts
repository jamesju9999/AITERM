import { useCallback, useEffect, useRef, useState } from "react";
import { vcsAgentStep, vcsQuery, type VcsAgentHistoryEntry, type VcsRepoInfo, type VcsResult } from "../ipc/vcs";
import { getConfig } from "../ipc/config";

export type VcsLoopMessageKind =
  | "user"
  | "step"
  | "step-loading"
  | "final-answer"
  | "step-limit-reached"
  | "stopped"
  | "error";

export interface VcsLoopMessage {
  id: string;
  kind: VcsLoopMessageKind;
  // user
  text?: string;
  // step
  stepNum?: number;
  maxSteps?: number;
  commandDisplay?: string;
  aiSummary?: string;
  result?: VcsResult;
  // final-answer / step-limit-reached / stopped / error
  content?: string;
}

export interface UseVcsAgentLoopResult {
  messages: VcsLoopMessage[];
  isRunning: boolean;
  send: (text: string, providerId?: string | null) => void;
  stop: () => void;
}

export function useVcsAgentLoop(sessionId: string, repoInfo: VcsRepoInfo | null): UseVcsAgentLoopResult {
  const [messages, setMessages] = useState<VcsLoopMessage[]>([]);
  const [isRunning, setIsRunning] = useState(false);

  const mountedRef = useRef(true);
  useEffect(() => {
    mountedRef.current = true;
    return () => { mountedRef.current = false; };
  }, []);

  // Reset messages when repo root changes
  const prevRootRef = useRef<string | null>(null);
  useEffect(() => {
    const root = repoInfo?.root ?? null;
    if (root !== prevRootRef.current) {
      prevRootRef.current = root;
      setMessages([]);
    }
  }, [repoInfo?.root]);

  // Refs for loop control
  const stopFlagRef = useRef(false);
  const pendingUserMsgRef = useRef<string | null>(null);

  const runLoop = useCallback(async (
    goal: string,
    initialHistory: VcsAgentHistoryEntry[],
    maxSteps: number,
    providerId: string | null | undefined,
    currentRepoInfo: VcsRepoInfo,
  ) => {
    if (!mountedRef.current) return;
    setIsRunning(true);
    stopFlagRef.current = false;

    const history: VcsAgentHistoryEntry[] = [...initialHistory];
    let stepCount = 0;

    try {
      while (true) {
        if (!mountedRef.current) break;

        // Check stop flag
        if (stopFlagRef.current) {
          setMessages((prev) => [
            ...prev,
            { id: crypto.randomUUID(), kind: "stopped", content: "已停止" },
          ]);
          break;
        }

        // Check step limit
        if (maxSteps > 0 && stepCount >= maxSteps) {
          setMessages((prev) => [
            ...prev,
            {
              id: crypto.randomUUID(),
              kind: "step-limit-reached",
              content: `已達步驟上限（${maxSteps} 步）`,
              maxSteps,
            },
          ]);
          break;
        }

        stepCount++;
        const loadingId = crypto.randomUUID();

        // Show loading state for this step
        setMessages((prev) => [
          ...prev,
          { id: loadingId, kind: "step-loading", stepNum: stepCount, maxSteps, content: `Step ${stepCount} 執行中…` },
        ]);

        // Call AI for next decision
        let decision;
        try {
          decision = await vcsAgentStep(goal, history, currentRepoInfo, sessionId, providerId);
        } catch (e) {
          if (!mountedRef.current) break;
          setMessages((prev) =>
            prev.map((m) =>
              m.id === loadingId
                ? { ...m, kind: "error" as VcsLoopMessageKind, content: `AI 無法規劃下一步：${String(e)}` }
                : m
            )
          );
          break;
        }

        if (!mountedRef.current) break;

        // Handle done=true
        if (decision.done) {
          setMessages((prev) =>
            prev.map((m) =>
              m.id === loadingId
                ? { ...m, kind: "final-answer" as VcsLoopMessageKind, content: decision.final_answer ?? decision.summary }
                : m
            )
          );
          break;
        }

        // Execute the intent
        let result: VcsResult;
        const intentJson = JSON.stringify(decision.intent);
        const operationName = (decision.intent as { kind?: string })?.kind ?? "unknown";

        try {
          // vcsQuery accepts a natural language string, but for agent loop we pass
          // a special prefix so the backend knows to skip re-parsing.
          // Actually: we need to call vcs_query but pass the intent directly.
          // Looking at the backend, vcs_query parses natural language → VcsIntent.
          // For the agent loop, the AI already decided the intent.
          // We'll pass a special "__intent__:" prefix to signal pre-parsed intent.
          // But wait — the backend vcs_query doesn't support that.
          //
          // Actually the simplest approach: we pass the JSON intent as the query,
          // and the existing parse_vcs_intent will see it's already JSON and parse it.
          // The existing strip_json_fences + serde_json::from_str<VcsIntent> should work.
          result = await vcsQuery(intentJson, currentRepoInfo, sessionId);
        } catch (e) {
          result = { type: "error", message: String(e) };
        }

        if (!mountedRef.current) break;

        const resultJson = JSON.stringify(result);

        // Determine commandDisplay from operation name
        const commandDisplay = operationName.replace(/_/g, " ");

        // Replace loading with step result
        setMessages((prev) =>
          prev.map((m) =>
            m.id === loadingId
              ? {
                  ...m,
                  kind: "step" as VcsLoopMessageKind,
                  stepNum: stepCount,
                  maxSteps,
                  commandDisplay,
                  aiSummary: decision.summary,
                  result,
                }
              : m
          )
        );

        // Append to history
        history.push({
          role: "step",
          step_num: stepCount,
          operation: operationName,
          result_json: resultJson,
          summary: decision.summary,
        });

        // Inject any pending user message
        const pendingMsg = pendingUserMsgRef.current;
        if (pendingMsg) {
          pendingUserMsgRef.current = null;
          history.push({ role: "user", text: pendingMsg });
        }
      }
    } finally {
      if (mountedRef.current) {
        setIsRunning(false);
      }
    }
  }, [sessionId]);

  const send = useCallback((text: string, providerId?: string | null) => {
    if (!repoInfo) return;

    if (isRunning) {
      // Inject user message into running loop
      const userMsg: VcsLoopMessage = {
        id: crypto.randomUUID(),
        kind: "user",
        text,
      };
      setMessages((prev) => [...prev, userMsg]);
      pendingUserMsgRef.current = text;
      return;
    }

    // Start new loop
    setMessages([{
      id: crypto.randomUUID(),
      kind: "user",
      text,
    }]);
    pendingUserMsgRef.current = null;

    getConfig().then((cfg) => {
      const maxSteps = cfg.max_agent_steps === 0 ? 9999 : (cfg.max_agent_steps ?? 5);
      const initialHistory: VcsAgentHistoryEntry[] = [{ role: "user", text }];
      void runLoop(text, initialHistory, maxSteps, providerId, repoInfo);
    }).catch(() => {
      const maxSteps = 5;
      const initialHistory: VcsAgentHistoryEntry[] = [{ role: "user", text }];
      void runLoop(text, initialHistory, maxSteps, providerId, repoInfo);
    });
  }, [isRunning, repoInfo, runLoop]);

  const stop = useCallback(() => {
    stopFlagRef.current = true;
  }, []);

  return { messages, isRunning, send, stop };
}
