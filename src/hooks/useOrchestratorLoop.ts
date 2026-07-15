import { useState, useCallback, useRef } from "react";
import { agentChat, type AgentToolDefinition, type ChatMessage } from "../ipc/ai";
import { runSubAgent, runToolLoop, serializeError, type AgentDefinition, type SubAgentAction } from "./useSubAgentLoop";
import { loopSessionSave, loopSessionLoad, parseLoopSessionData } from "../ipc/loopSession";
import { languageDirective, type Locale } from "../lib/i18n";

export interface OrchestratorAgent extends AgentDefinition {
  isOrchestrator?: boolean;
  isVerifier?: boolean;
}

export interface LoopConfig {
  goal: string;
  stoppingCondition: string;
  orchestrator: OrchestratorAgent;
  verifier: OrchestratorAgent;
  subAgents: OrchestratorAgent[];
  maxLoops: number;
  /** 0 = unlimited */
  maxOrchestratorSteps: number;
  /** 0 = unlimited */
  maxInnerIterations: number;
  sessionId: string;
  /** Absolute path all sub-agents should treat as the working directory */
  projectDir?: string;
  /** true = 跳過危險指令確認，全自動執行 */
  fullAuto?: boolean;
  /** UI locale at the time the loop was started — controls AI-bound prompt language */
  locale?: Locale;
  loopSessionId?: string;
  resumeSnapshot?: {
    orchestratorHistory: ChatMessage[];
    sharedContext: string;
    trace: TraceEntry[];
    startIteration: number;
  };
}

export type TraceEntryKind =
  | "iteration_start"
  | "orchestrator_action"
  | "sub_agent_start"
  | "sub_agent_action"
  | "sub_agent_done"
  | "verifier_result"
  | "loop_done"
  | "loop_stopped"
  | "loop_error";

export interface VerifierResult {
  done: boolean;
  summary: string;
  accomplished: string[];
  remaining: string[];
  suggestion: string;
}

export interface TraceEntry {
  id: string;
  kind: TraceEntryKind;
  iteration?: number;
  agentName?: string;
  text: string;
  actions?: SubAgentAction[];
  verifierDone?: boolean;
  verifierResult?: VerifierResult;
  isError?: boolean;
  timestamp: number;
  startTimestamp?: number;  // 對應區塊的開始時間（毫秒）
}

export interface PendingConfirmation {
  agentName: string;
  command: string;
  resolve: (approved: boolean) => void;
}

export interface UseOrchestratorLoopResult {
  trace: TraceEntry[];
  isRunning: boolean;
  iteration: number;
  start: (config: LoopConfig) => Promise<void>;
  stop: () => void;
  resume: (sessionId: string) => Promise<void>;
  pendingConfirmation: PendingConfirmation | null;
}

function buildCallAgentTool(subAgents: OrchestratorAgent[]): AgentToolDefinition {
  const agentNames = subAgents.map(a => a.name);
  const agentNameField = agentNames.length > 0
    ? { type: "string", enum: agentNames, description: `The sub-agent to delegate to. Must be one of: ${agentNames.join(", ")}` }
    : { type: "string", description: "Name of the sub-agent to call" };

  return {
    name: "call_agent",
    description: "Delegate a task to a specialized sub-agent. The sub-agent will complete the task and return the result.",
    input_schema: {
      type: "object",
      properties: {
        agent_name: agentNameField,
        task: { type: "string", description: "Detailed task description for the sub-agent to complete" },
      },
      required: ["agent_name", "task"],
    },
  };
}

function buildOrchestratorSystemPrompt(config: LoopConfig, sharedContext: string, locale: Locale): string {
  const agentDescriptions = config.subAgents.map(a => {
    const toolList = a.tools.length > 0 ? a.tools.join(", ") : "none";
    return `- ${a.name}: ${a.roleDescription} (tools: ${toolList})`;
  }).join("\n");

  const contextSection = sharedContext
    ? `\n## Accumulated Context From Previous Iterations\n${sharedContext}\n`
    : "";

  const dirSection = config.projectDir
    ? `\n## Working Directory\nAll file operations and commands must be performed under: ${config.projectDir}\nAlways instruct sub-agents to work within this directory.\n`
    : "";

  return `You are an Orchestrator AI.

## Goal
${config.goal}
${dirSection}${contextSection}
## Available Sub-Agents
${agentDescriptions}

## Instructions
1. Review the accumulated context above to understand what has already been done and what remains.
2. Delegate tasks to sub-agents using the call_agent tool. Avoid repeating work already completed.
3. When all sub-tasks are done, respond with a final summary WITHOUT calling any tools.
4. Always write your final response in ${languageDirective(locale)}.`;
}

function buildVerifierSystemPrompt(stoppingCondition: string, subAgentNames: string[], locale: Locale): string {
  const agentList = subAgentNames.length > 0
    ? subAgentNames.map(n => `"${n}"`).join(", ")
    : "(no agents available)";

  return `You are a Verifier AI. Your job is to objectively evaluate progress toward a goal.

## Stopping Condition
${stoppingCondition}

## Available Sub-Agents (ONLY use these names in your suggestion)
${agentList}

## Verification Tools
You have read-only tools (read_file, list_directory). Before concluding, you may
use them to inspect the actual files and verify the Orchestrator's claims.

## Instructions
Analyze the Orchestrator's report and respond ONLY with a JSON object in this exact format:
{
  "done": true or false,
  "summary": "one-sentence summary of overall progress so far",
  "accomplished": ["specific completed item 1", "specific completed item 2"],
  "remaining": ["specific incomplete item 1", "specific incomplete item 2"],
  "suggestion": "concrete next-step suggestion for the Orchestrator, using only the agent names listed above"
}

Rules:
- "accomplished" and "remaining" must be specific and verifiable, not vague
- "suggestion" MUST only reference agents from the available list above — never invent new agent names
- If done is true, "remaining" should be empty and "suggestion" can be empty
- Write all values in ${languageDirective(locale)}
- Do NOT include any text outside the JSON object`;
}

function buildSharedContextUpdate(iter: number, result: VerifierResult, subAgentSummaries: string[]): string {
  const lines = [`### Iteration #${iter} Result`];
  if (subAgentSummaries.length > 0) {
    lines.push("**Agent Execution Summary:**");
    subAgentSummaries.forEach(s => lines.push(`  - ${s}`));
  }
  if (result.accomplished.length > 0) {
    lines.push("**Accomplished:**");
    result.accomplished.forEach(a => lines.push(`  ✓ ${a}`));
  }
  if (result.remaining.length > 0) {
    lines.push("**Remaining:**");
    result.remaining.forEach(r => lines.push(`  ✗ ${r}`));
  }
  return lines.join("\n");
}

function traceId(): string {
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function parseVerifierResult(raw: string): VerifierResult | null {
  try {
    // Strip markdown code fences if present
    const cleaned = raw.trim().replace(/^```(?:json)?\n?/, "").replace(/\n?```$/, "");
    const parsed = JSON.parse(cleaned) as Partial<VerifierResult>;
    return {
      done: Boolean(parsed.done),
      summary: parsed.summary ?? "",
      accomplished: Array.isArray(parsed.accomplished) ? parsed.accomplished : [],
      remaining: Array.isArray(parsed.remaining) ? parsed.remaining : [],
      suggestion: parsed.suggestion ?? "",
    };
  } catch {
    return null;
  }
}

export function useOrchestratorLoop(): UseOrchestratorLoopResult {
  const [trace, setTrace] = useState<TraceEntry[]>([]);
  const [isRunning, setIsRunning] = useState(false);
  const [iteration, setIteration] = useState(0);
  const abortRef = useRef(false);
  const [pendingConfirmation, setPendingConfirmation] = useState<PendingConfirmation | null>(null);
  const pendingResolveRef = useRef<((approved: boolean) => void) | null>(null);

  const requestConfirmation = useCallback((agentName: string, command: string): Promise<boolean> => {
    return new Promise<boolean>((resolve) => {
      const wrapped = (approved: boolean) => {
        setPendingConfirmation(null);
        pendingResolveRef.current = null;
        resolve(approved);
      };
      pendingResolveRef.current = wrapped;
      setPendingConfirmation({ agentName, command, resolve: wrapped });
    });
  }, []);

  const addTrace = useCallback((entry: Omit<TraceEntry, "id" | "timestamp">) => {
    setTrace(prev => [...prev, { ...entry, id: traceId(), timestamp: Date.now() }]);
  }, []);

  const stop = useCallback(() => {
    abortRef.current = true;
    pendingResolveRef.current?.(false);
  }, []);

  const start = useCallback(async (config: LoopConfig) => {
    abortRef.current = false;
    const locale: Locale = config.locale ?? "zh-TW";
    const loopSessionId = config.loopSessionId ?? crypto.randomUUID();

    const traceBuffer: TraceEntry[] = config.resumeSnapshot?.trace ? [...config.resumeSnapshot.trace] : [];
    const addTraceBuffered = (entry: Omit<TraceEntry, "id" | "timestamp">) => {
      const full: TraceEntry = { ...entry, id: traceId(), timestamp: Date.now() };
      traceBuffer.push(full);
      setTrace(prev => [...prev, full]);
    };

    setTrace(traceBuffer);
    setIteration(config.resumeSnapshot?.startIteration ?? 1);
    setIsRunning(true);

    const maxLoops = config.maxLoops === 0 ? 20 : config.maxLoops;
    const callAgentTool = buildCallAgentTool(config.subAgents);

    // Shared context accumulates across all iterations
    let sharedContext = config.resumeSnapshot?.sharedContext ?? "";

    const orchestratorHistory: ChatMessage[] = config.resumeSnapshot?.orchestratorHistory ?? [
      { role: "system", content: buildOrchestratorSystemPrompt(config, sharedContext, locale) },
      { role: "user", content: `Begin working toward the goal: ${config.goal}` },
    ];

    const saveSnapshot = (status: "running" | "paused" | "completed" | "failed", currentIter: number) => {
      void loopSessionSave(
        loopSessionId,
        config.goal,
        status,
        currentIter,
        { config, orchestratorHistory, sharedContext, trace: traceBuffer },
      ).catch((err) => console.error("[Loop] saveSnapshot failed:", err));
    };

    try {
      // --- Preflight: verify Orchestrator can call tools before starting the loop ---
      addTraceBuffered({
        kind: "orchestrator_action",
        agentName: config.orchestrator.name,
        text: "🔍 正在測試 Orchestrator 工具呼叫能力...",
        iteration: 0,
      });

      const agentNames = config.subAgents.map(a => a.name);
      const preflightMessages: ChatMessage[] = [
        { role: "system", content: `You are an Orchestrator AI. You must use the call_agent tool to delegate tasks. Available agents: ${agentNames.join(", ")}.` },
        { role: "user", content: `[Preflight test] Immediately call the call_agent tool, assigning any simple task to any available agent. Call the tool directly — do not output any explanatory text.` },
      ];

      let preflightPassed = false;
      let preflightError = "";

      try {
        const preflightReply = await agentChat(
          config.orchestrator.providerId,
          preflightMessages,
          [callAgentTool],
          config.sessionId,
        );

        if (preflightReply.tool_calling_unsupported) {
          preflightError = `模型不支援 Tool Calling。請改用支援 Function Calling 的模型（如 GPT-4o、Claude 3.5+、Gemini 1.5 Pro+）。`;
        } else if (preflightReply.tool_calls.some(tc => tc.tool_name === "call_agent")) {
          preflightPassed = true;
        } else {
          preflightError = `Orchestrator 回傳了文字而非工具呼叫。此模型可能無法可靠地驅動 Multi-Agent Loop。`;
        }
      } catch (err) {
        preflightError = err instanceof Error ? err.message : JSON.stringify(err);
      }

      if (!preflightPassed) {
        addTraceBuffered({
          kind: "loop_stopped",
          text: `✗ 前置測試失敗：${preflightError}\n提示：可在設定中切換 Orchestrator 模型後重試。`,
          isError: true,
        });
        setIsRunning(false);
        return;
      }

      addTraceBuffered({
        kind: "orchestrator_action",
        agentName: config.orchestrator.name,
        text: "✓ 工具呼叫前置測試通過，開始執行 Loop",
        iteration: 0,
      });

      const startIter = config.resumeSnapshot?.startIteration ?? 1;
      saveSnapshot("running", 0);

      // Repetition detector: track last N (agent, task) pairs to catch infinite loops
      const REPEAT_LIMIT = 3;
      const recentCalls: string[] = [];

      for (let iter = startIter; iter <= maxLoops; iter++) {
        if (abortRef.current) {
          addTraceBuffered({ kind: "loop_stopped", text: "⊘ 已被使用者停止", iteration: iter });
          saveSnapshot("paused", iter);
          break;
        }

        // Update orchestrator system prompt with latest shared context each iteration
        if (iter > 1 && sharedContext) {
          orchestratorHistory[0] = {
            role: "system",
            content: buildOrchestratorSystemPrompt(config, sharedContext, locale),
          };
        }

        setIteration(iter);
        addTraceBuffered({ kind: "iteration_start", text: `Loop #${iter}`, iteration: iter });

        let orchestratorFinalAnswer = "";
        let noToolCallRetries = 0;
        const orchLimit = config.maxOrchestratorSteps === 0 ? Infinity : config.maxOrchestratorSteps;
        const iterSubAgentSummaries: string[] = [];

        for (let step = 0; step < orchLimit; step++) {
          if (abortRef.current) break;

          const reply = await agentChat(
            config.orchestrator.providerId,
            orchestratorHistory,
            [callAgentTool],
            config.sessionId,
          );


          if (reply.tool_calling_unsupported) {
            addTraceBuffered({
              kind: "loop_error",
              text: `⚠ Orchestrator 使用的 Provider 不支援 Tool Calling，無法呼叫 Agent。請在設定中改用支援 Function Calling 的模型（如 GPT-4o、Claude、Gemini 1.5 Pro）。`,
              isError: true,
            });
            abortRef.current = true;
            break;
          }

          if (reply.tool_calls.length === 0) {
            const responseText = reply.content ?? "";

            // If tasks are incomplete and the model produced text without calling any agent,
            // it's likely "planning text" rather than a real final answer.
            // Inject a correction and retry (up to 2 times per iteration).
            const agentNames = config.subAgents.map(a => a.name);
            const looksLikePlan = agentNames.some(n => responseText.includes(n)) ||
              /應該|需要|建議|請|should|need to|will now|let me|plan to|call_agent|delegate|assign/i.test(responseText) ||
              responseText.includes("<tool_call>");

            if (looksLikePlan && noToolCallRetries < 2) {
              noToolCallRetries++;
              orchestratorHistory.push({ role: "assistant", content: responseText });
              orchestratorHistory.push({
                role: "user",
                content: `You just described a plan but did not call the call_agent tool. You MUST immediately use the call_agent tool to delegate a task to a sub-agent — do not output any more explanatory text. Available agents: ${agentNames.join(", ")}.`,
              });
              addTraceBuffered({
                kind: "orchestrator_action",
                agentName: config.orchestrator.name,
                text: `[重試 ${noToolCallRetries}/2] Orchestrator 輸出了文字但未呼叫工具，已注入糾正提示`,
                iteration: iter,
              });
              continue;
            }

            // Genuine final answer (or exhausted retries)
            noToolCallRetries = 0;
            orchestratorFinalAnswer = responseText;
            addTraceBuffered({ kind: "orchestrator_action", agentName: config.orchestrator.name, text: orchestratorFinalAnswer, iteration: iter });
            orchestratorHistory.push({ role: "assistant", content: orchestratorFinalAnswer });
            break;
          }

          noToolCallRetries = 0;

          for (const tc of reply.tool_calls) {
            if (tc.tool_name !== "call_agent") continue;
            if (abortRef.current) break;

            // Normalize args — some providers use alternative field names
            const rawArgs = tc.args as Record<string, unknown>;
            const agentName = (rawArgs.agent_name ?? rawArgs.agent ?? rawArgs.name ?? "") as string;
            const task = (
              rawArgs.task ??
              rawArgs.description ??
              rawArgs.instruction ??
              (Array.isArray(rawArgs.assignments) && rawArgs.assignments.length > 0
                ? (rawArgs.assignments[0] as Record<string, unknown>).description ?? JSON.stringify(rawArgs.assignments[0])
                : undefined) ??
              ""
            ) as string;
            const args = { agent_name: agentName, task };

            const targetAgent = config.subAgents.find(a => a.name === args.agent_name);

            // Repetition detection: same (agent, task-prefix) repeated REPEAT_LIMIT times = stuck loop
            const callKey = `${args.agent_name}::${args.task.slice(0, 120)}`;
            recentCalls.push(callKey);
            if (recentCalls.length > REPEAT_LIMIT) recentCalls.shift();
            if (recentCalls.length === REPEAT_LIMIT && recentCalls.every(k => k === callKey)) {
              addTraceBuffered({
                kind: "loop_stopped",
                text: `⚠ 偵測到重複迴圈：${args.agent_name} 連續 ${REPEAT_LIMIT} 次接收到完全相同的任務，但沒有實際進展。這通常代表 Sub-agent 模型無法正確呼叫工具（只輸出文字而非執行），請改用支援 Function Calling 的模型（如 GPT-4o、Claude 3.5+、Gemini 1.5 Pro、Qwen2.5-7B-Instruct 等）。`,
                isError: true,
                iteration: iter,
              });
              saveSnapshot("failed", iter);
              abortRef.current = true;
              break;
            }

            addTraceBuffered({
              kind: "orchestrator_action",
              agentName: config.orchestrator.name,
              text: `→ 委派給 ${args.agent_name}：${args.task.slice(0, 200)}${args.task.length > 200 ? "..." : ""}`,
              iteration: iter,
            });
            addTraceBuffered({
              kind: "sub_agent_start",
              agentName: args.agent_name,
              text: `${args.agent_name}: ${args.task}`,
              iteration: iter,
            });

            let subResult: string;
            if (!targetAgent) {
              const available = config.subAgents.map(a => `"${a.name}"`).join(", ");
              subResult = `Error: agent "${args.agent_name}" does not exist in the roster. Available agents: ${available}. Please use one of these instead.`;
              addTraceBuffered({ kind: "sub_agent_done", agentName: args.agent_name, text: subResult, isError: true, iteration: iter });
            } else {
              const confirmFn = config.fullAuto
                ? async () => true
                : (command: string) => requestConfirmation(args.agent_name, command);
              const agentStartTs = Date.now();
              const result = await runSubAgent(
                config.sessionId,
                targetAgent,
                args.task,
                {
                  onAction: (action) => addTraceBuffered({ kind: "sub_agent_action", agentName: args.agent_name, text: action.tool, actions: [action], iteration: iter }),
                  onConfirmNeeded: confirmFn,
                  maxInnerIterations: config.maxInnerIterations,
                  sharedContext,
                  projectDir: config.projectDir,
                  locale,
                },
              );
              subResult = result.answer;
              iterSubAgentSummaries.push(`${args.agent_name} → ${subResult.slice(0, 100)}${subResult.length > 100 ? "..." : ""}`);
              addTraceBuffered({
                kind: "sub_agent_done",
                agentName: args.agent_name,
                text: subResult,
                actions: result.actions,
                isError: result.isError,
                iteration: iter,
                startTimestamp: agentStartTs,
              });
            }

            // Store tool call result in proper OpenAI format so the Orchestrator
            // can correctly read what each agent reported back.
            // Use raw_tool_calls verbatim (preserves Gemini thought_signature); fall back to reconstruction.
            const toolCallId = tc.id || `call_${Date.now()}`;
            const rawToolCalls = reply.raw_tool_calls;
            orchestratorHistory.push({
              role: "assistant",
              content: null,
              tool_calls: rawToolCalls ?? [{
                id: toolCallId,
                type: "function",
                function: { name: "call_agent", arguments: JSON.stringify(args) },
              }],
            });
            orchestratorHistory.push({
              role: "tool",
              content: subResult,
              tool_call_id: toolCallId,
            });
          }
        }

        if (abortRef.current) {
          addTraceBuffered({ kind: "loop_stopped", text: "⊘ 已被使用者停止", iteration: iter });
          saveSnapshot("paused", iter);
          break;
        }

        // Run Verifier with full context
        const verifierMessages: ChatMessage[] = [
          { role: "system", content: buildVerifierSystemPrompt(config.stoppingCondition, config.subAgents.map(a => a.name), locale) },
          {
            role: "user",
            content: [
              `## Goal\n${config.goal}`,
              sharedContext ? `## Accumulated Context From Previous Iterations\n${sharedContext}` : "",
              `## Orchestrator Report This Round (Iteration #${iter})\n${orchestratorFinalAnswer}`,
            ].filter(Boolean).join("\n\n"),
          },
        ];

        const verifierStartTs = Date.now();
        const verifierRun = await runToolLoop(
          config.verifier.providerId,
          verifierMessages,
          ["read_file", "list_directory"],
          { sessionId: config.sessionId, effectiveRoot: config.projectDir ?? null },
          8,
          (action) => addTraceBuffered({ kind: "sub_agent_action", agentName: config.verifier.name, text: action.tool, actions: [action], iteration: iter }),
        );

        const verifierResult = parseVerifierResult(verifierRun.answer);

        if (!verifierResult) {
          addTraceBuffered({
            kind: "verifier_result",
            agentName: config.verifier.name,
            text: "⚠ Verifier 回應無法解析，視為未完成",
            verifierDone: false,
            iteration: iter,
            startTimestamp: verifierStartTs,
          });
          orchestratorHistory.push({
            role: "user",
            content: "The Verifier's response could not be parsed. Please continue trying to achieve the goal.",
          });
          continue;
        }

        // Update shared context with this iteration's findings
        sharedContext += (sharedContext ? "\n\n" : "") +
          buildSharedContextUpdate(iter, verifierResult, iterSubAgentSummaries);

        addTraceBuffered({
          kind: "verifier_result",
          agentName: config.verifier.name,
          text: verifierResult.summary,
          verifierDone: verifierResult.done,
          verifierResult,
          iteration: iter,
          startTimestamp: verifierStartTs,
        });
        saveSnapshot("running", iter);

        if (verifierResult.done) {
          addTraceBuffered({ kind: "loop_done", text: `✓ 目標達成：${verifierResult.summary}` });
          saveSnapshot("completed", iter);
          break;
        }

        // Feed structured feedback to orchestrator for next iteration
        const feedbackMsg = [
          `## Verifier Feedback (Iteration #${iter})`,
          `**Summary:** ${verifierResult.summary}`,
          verifierResult.accomplished.length > 0
            ? `**Accomplished:**\n${verifierResult.accomplished.map(a => `- ${a}`).join("\n")}`
            : "",
          verifierResult.remaining.length > 0
            ? `**Remaining:**\n${verifierResult.remaining.map(r => `- ${r}`).join("\n")}`
            : "",
          verifierResult.suggestion
            ? `**Next-step suggestion:** ${verifierResult.suggestion}`
            : "",
        ].filter(Boolean).join("\n\n");

        orchestratorHistory.push({ role: "user", content: feedbackMsg });

        if (iter === maxLoops) {
          addTraceBuffered({ kind: "loop_stopped", text: `⚠ 已達最大輪數 ${maxLoops}，停止`, iteration: iter });
        }
      }
    } catch (e) {
      addTraceBuffered({ kind: "loop_error", text: `錯誤：${serializeError(e)}`, isError: true });
      saveSnapshot("failed", 0);
    } finally {
      setIsRunning(false);
    }
  }, [addTrace, requestConfirmation]);

  const resume = useCallback(async (sessionId: string) => {
    const data = await loopSessionLoad(sessionId);
    const snap = parseLoopSessionData(data);
    await start({
      ...snap.config,
      loopSessionId: sessionId,
      resumeSnapshot: {
        orchestratorHistory: snap.orchestratorHistory,
        sharedContext: snap.sharedContext,
        trace: snap.trace,
        startIteration: snap.iteration + 1,
      },
    });
  }, [start]);

  return { trace, isRunning, iteration, start, stop, resume, pendingConfirmation };
}
