import { useState, useCallback, useRef } from "react";
import { agentChat, type AgentToolDefinition, type ChatMessage } from "../ipc/ai";
import { runSubAgent, serializeError, type AgentDefinition, type SubAgentAction } from "./useSubAgentLoop";
import { loopSessionSave, loopSessionLoad, parseLoopSessionData } from "../ipc/loopSession";

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
}

export interface UseOrchestratorLoopResult {
  trace: TraceEntry[];
  isRunning: boolean;
  iteration: number;
  start: (config: LoopConfig) => Promise<void>;
  stop: () => void;
  resume: (sessionId: string) => Promise<void>;
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

function buildOrchestratorSystemPrompt(config: LoopConfig, sharedContext: string): string {
  const agentDescriptions = config.subAgents.map(a => {
    const toolList = a.tools.length > 0 ? a.tools.join(", ") : "none";
    return `- ${a.name}: ${a.roleDescription} (tools: ${toolList})`;
  }).join("\n");

  const contextSection = sharedContext
    ? `\n## 先前迭代的累積 Context\n${sharedContext}\n`
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
3. When all sub-tasks are done, respond with a final summary in Traditional Chinese WITHOUT calling any tools.
4. Always write your final response in Traditional Chinese (繁體中文).`;
}

function buildVerifierSystemPrompt(stoppingCondition: string, subAgentNames: string[]): string {
  const agentList = subAgentNames.length > 0
    ? subAgentNames.map(n => `"${n}"`).join("、")
    : "（無可用 agent）";

  return `You are a Verifier AI. Your job is to objectively evaluate progress toward a goal.

## Stopping Condition
${stoppingCondition}

## Available Sub-Agents (ONLY use these names in your suggestion)
${agentList}

## Instructions
Analyze the Orchestrator's report and respond ONLY with a JSON object in this exact format:
{
  "done": true or false,
  "summary": "一句話總結目前整體進度",
  "accomplished": ["具體已完成的事項1", "具體已完成的事項2"],
  "remaining": ["具體尚未完成的事項1", "具體尚未完成的事項2"],
  "suggestion": "給 Orchestrator 的具體下一步行動建議，只能使用上方列出的 agent 名稱"
}

Rules:
- "accomplished" and "remaining" must be specific and verifiable, not vague
- "suggestion" MUST only reference agents from the available list above — never invent new agent names
- If done is true, "remaining" should be empty and "suggestion" can be empty
- Write all values in Traditional Chinese (繁體中文)
- Do NOT include any text outside the JSON object`;
}

function buildSharedContextUpdate(iter: number, result: VerifierResult, subAgentSummaries: string[]): string {
  const lines = [`### 迭代 #${iter} 結果`];
  if (subAgentSummaries.length > 0) {
    lines.push("**Agent 執行摘要：**");
    subAgentSummaries.forEach(s => lines.push(`  - ${s}`));
  }
  if (result.accomplished.length > 0) {
    lines.push("**已完成：**");
    result.accomplished.forEach(a => lines.push(`  ✓ ${a}`));
  }
  if (result.remaining.length > 0) {
    lines.push("**尚未完成：**");
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

  const addTrace = useCallback((entry: Omit<TraceEntry, "id" | "timestamp">) => {
    setTrace(prev => [...prev, { ...entry, id: traceId(), timestamp: Date.now() }]);
  }, []);

  const stop = useCallback(() => {
    abortRef.current = true;
  }, []);

  const start = useCallback(async (config: LoopConfig) => {
    abortRef.current = false;
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
      { role: "system", content: buildOrchestratorSystemPrompt(config, sharedContext) },
      { role: "user", content: `請開始執行目標：${config.goal}` },
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
        { role: "system", content: `你是一個 Orchestrator AI。你必須使用 call_agent 工具委派任務。可用的 Agent：${agentNames.join("、")}。` },
        { role: "user", content: `[前置測試] 請立即呼叫 call_agent 工具，指派任意一個簡單任務給任意一個可用的 Agent。直接呼叫工具，不要輸出任何文字說明。` },
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
            content: buildOrchestratorSystemPrompt(config, sharedContext),
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
              /應該|需要|建議|請|call_agent|delegate|assign/i.test(responseText) ||
              responseText.includes("<tool_call>");

            if (looksLikePlan && noToolCallRetries < 2) {
              noToolCallRetries++;
              orchestratorHistory.push({ role: "assistant", content: responseText });
              orchestratorHistory.push({
                role: "user",
                content: `你剛才描述了計畫但沒有呼叫 call_agent 工具。請「立即」使用 call_agent 工具委派任務給 Sub-agent，不要再輸出文字說明。可用的 Agent：${agentNames.join("、")}。`,
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
              const available = config.subAgents.map(a => `"${a.name}"`).join("、");
              subResult = `錯誤：Agent "${args.agent_name}" 不存在於 Roster 中。目前可用的 Agent 只有：${available}。請改用其中一個。`;
              addTraceBuffered({ kind: "sub_agent_done", agentName: args.agent_name, text: subResult, isError: true, iteration: iter });
            } else {
              const result = await runSubAgent(
                config.sessionId,
                targetAgent,
                args.task,
                (action) => addTraceBuffered({ kind: "sub_agent_action", agentName: args.agent_name, text: action.tool, actions: [action], iteration: iter }),
                config.maxInnerIterations,
                sharedContext,
                config.projectDir,
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
          { role: "system", content: buildVerifierSystemPrompt(config.stoppingCondition, config.subAgents.map(a => a.name)) },
          {
            role: "user",
            content: [
              `## 目標\n${config.goal}`,
              sharedContext ? `## 先前迭代的累積 Context\n${sharedContext}` : "",
              `## 本輪 Orchestrator 報告（迭代 #${iter}）\n${orchestratorFinalAnswer}`,
            ].filter(Boolean).join("\n\n"),
          },
        ];

        const verifierReply = await agentChat(
          config.verifier.providerId,
          verifierMessages,
          [],
          config.sessionId,
        );

        const verifierResult = parseVerifierResult(verifierReply.content ?? "");

        if (!verifierResult) {
          addTraceBuffered({
            kind: "verifier_result",
            agentName: config.verifier.name,
            text: "⚠ Verifier 回應無法解析，視為未完成",
            verifierDone: false,
            iteration: iter,
          });
          orchestratorHistory.push({
            role: "user",
            content: "Verifier 無法解析回應，請繼續嘗試達成目標。",
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
        });
        saveSnapshot("running", iter);

        if (verifierResult.done) {
          addTraceBuffered({ kind: "loop_done", text: `✓ 目標達成：${verifierResult.summary}` });
          saveSnapshot("completed", iter);
          break;
        }

        // Feed structured feedback to orchestrator for next iteration
        const feedbackMsg = [
          `## Verifier 反饋（迭代 #${iter}）`,
          `**總結：** ${verifierResult.summary}`,
          verifierResult.accomplished.length > 0
            ? `**已完成：**\n${verifierResult.accomplished.map(a => `- ${a}`).join("\n")}`
            : "",
          verifierResult.remaining.length > 0
            ? `**尚未完成：**\n${verifierResult.remaining.map(r => `- ${r}`).join("\n")}`
            : "",
          verifierResult.suggestion
            ? `**下一步建議：** ${verifierResult.suggestion}`
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
  }, [addTrace]);

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

  return { trace, isRunning, iteration, start, stop, resume };
}
