import type React from "react";
import type { Terminal } from "@xterm/xterm";
import type { ExecutionMode } from "../ipc/config";
import { formatAiError, type AiCommandReady, type AiError, type RiskLevel } from "../ipc/ai";
import type { AgentPhase } from "../components/AgentStatusBar";
import type { AgentStepInfo } from "./agentStepReport";
import type { TerminalBlock } from "../hooks/useTerminalBlocks";
import { webSearch, webFetch } from "../ipc/web";
import { repairUnterminatedHeredocs } from "./heredocGuard";

interface PreviewState {
  loading: boolean;
  visible: boolean;
  command: string;
  explanation: string;
  riskLevel: RiskLevel;
}

const INITIAL_PREVIEW: PreviewState = {
  loading: false,
  visible: false,
  command: "",
  explanation: "",
  riskLevel: "safe",
};

/** Decide whether to auto-execute based on execution mode and risk level. */
function shouldAutoExecute(mode: ExecutionMode, risk: RiskLevel, agentActive = false): boolean {
  // When the agent loop is active, be more aggressive to keep the loop autonomous
  if (agentActive) {
    if (risk === "safe") return true;                                    // Always auto-exec safe in agent mode
    if (risk === "needs_confirm" && mode === "full-auto") return true;   // Full-auto agent: also auto-exec needs_confirm
    if (risk === "dangerous") return false;                              // Dangerous always requires manual confirmation
  }
  if (mode === "always-confirm") return false;
  if (mode === "graded") return risk === "safe";
  if (mode === "full-auto") return risk === "safe" || risk === "needs_confirm";
  return false;
}

/**
 * Kick off a single /ai request: show the streaming indicator,
 * invoke the backend, then either auto-execute or show the preview.
 * For the agent loop, this is called with agentActive=true and onCommandComplete
 * which fires AFTER the executed command finishes in the PTY.
 */
function handleAiQuery(
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  t: any,
  queryFn: (query: string) => Promise<AiCommandReady>,
  originalLine: string,
  query: string,
  term: Terminal,
  setPreview: (p: PreviewState) => void,
  setStreamText: React.Dispatch<React.SetStateAction<string>>,
  streamingRef: React.MutableRefObject<boolean>,
  executionModeRef: React.MutableRefObject<ExecutionMode>,
  writeRed: (msg: string) => void,
  submitCommand: (cmd: string, onComplete?: (block: TerminalBlock) => void) => void,
  onDone?: (explanation?: string) => void,
  agentActive = false,
  onCommandComplete?: (block: TerminalBlock) => void,
  onAiError?: (err: AiError) => void,
  onWebAction?: (type: "search" | "fetch", value: string) => void,
  onPhase?: (update: AgentPhase) => void,
  agentStep = 0,
  agentMaxSteps = 0,
) {
  void originalLine;
  setStreamText("");
  streamingRef.current = true;
  setPreview({ loading: true, visible: false, command: "", explanation: "", riskLevel: "safe" });

  queryFn(query)
    .then((resp) => {
      streamingRef.current = false;

      if (resp.command === "DONE") {
        setPreview(INITIAL_PREVIEW);
        if (onDone) onDone(resp.explanation);
        return;
      }

      // Intercept web search/fetch commands before PTY execution
      if (resp.command.startsWith("AITERM_WEB_SEARCH: ") && onWebAction) {
        const value = resp.command.slice("AITERM_WEB_SEARCH: ".length);
        setPreview(INITIAL_PREVIEW);
        onWebAction("search", value);
        return;
      }
      if (resp.command.startsWith("AITERM_WEB_FETCH: ") && onWebAction) {
        const value = resp.command.slice("AITERM_WEB_FETCH: ".length);
        setPreview(INITIAL_PREVIEW);
        onWebAction("fetch", value);
        return;
      }

      const mode = executionModeRef.current;
      const risk = resp.risk_level;

      if (shouldAutoExecute(mode, risk, agentActive)) {
        // 沒人盯著的自動執行才補 heredoc 結束標記——缺了它 shell 會停在
        // heredoc> 等一個不會來的標記，指令永遠不結束、OSC 133 D 不發出，
        // 這個迴圈就在下面 onCommandComplete 那裡等到天荒地老。理由詳見
        // heredocGuard.ts。顯示的也是修補後的版本：畫面要跟實際跑的一致。
        const command = repairUnterminatedHeredocs(resp.command);
        // Auto-execute: write a subtle confirmation line then submit.
        const riskColor = risk === "safe" ? "\x1b[32m" : "\x1b[33m";
        term.write(`\r\n${riskColor}▶ ${command}\x1b[0m\r\n`);
        onPhase?.({ phase: "running", step: agentStep, maxSteps: agentMaxSteps, command });
        // Pass onCommandComplete so the block hook calls it when OSC 133;D fires
        submitCommand(command, onCommandComplete);
        setPreview(INITIAL_PREVIEW);
      } else {
        // Show preview with risk badge.
        if (risk === "dangerous") {
          term.write(`\x1b[31m${t.term_danger_warning}\x1b[0m\r\n`);
        }
        setPreview({
          loading: false,
          visible: true,
          command: resp.command,
          explanation: resp.explanation,
          riskLevel: risk,
        });
      }
    })
    .catch((rawErr: unknown) => {
      streamingRef.current = false;
      setStreamText("");
      const err = normalizeAiError(rawErr);
      writeRed(formatAiError(err));

      // Actionable follow-up hints
      if (err.kind === "not_configured") {
        term.write(`\x1b[33m${t.term_setup_hint_provider}\x1b[0m\r\n`);
      } else if (
        err.kind === "network" &&
        (err.message?.toLowerCase().includes("ollama") ||
          err.message?.toLowerCase().includes("connection refused"))
      ) {
        term.write(
          `\x1b[33m${t.term_setup_hint_ollama}\x1b[0m\r\n`
        );
      } else if (err.kind === "auth_failed") {
        term.write(`\x1b[33m${t.term_setup_hint_api_key}\x1b[0m\r\n`);
      }

      setPreview(INITIAL_PREVIEW);
      if (onAiError) onAiError(err);
    });
}

/**
 * Callback-driven Agent Loop.
 * Each step: ask AI → auto-execute command → wait for block completion → extract output → repeat.
 * This does NOT rely on React useEffect — the loop is driven by OSC 133;D completion callbacks.
 */
interface AgentLoopParams {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  t: any;
  goal: string;
  queryFn: (query: string) => Promise<AiCommandReady>;
  term: Terminal;
  getSubmitCommand: () => (cmd: string, onComplete?: (block: TerminalBlock) => void) => void;
  setPreview: (p: PreviewState) => void;
  setStreamText: React.Dispatch<React.SetStateAction<string>>;
  streamingRef: React.MutableRefObject<boolean>;
  executionModeRef: React.MutableRefObject<ExecutionMode>;
  writeRed: (msg: string) => void;
  abortRef: React.MutableRefObject<boolean>;
  stepCount: number;
  maxSteps: number;
  history: { command: string; exitCode: number; output: string }[];
  onComplete: (explanation?: string) => void;
  onFail: (msg: string) => void;
  /** Fires after each shell command finishes; lets the caller mirror progress (e.g. to Telegram). */
  onStepComplete?: (info: AgentStepInfo) => void;
  /** Pushes agent lifecycle status to the React status bar (replaces term.write status lines). */
  onPhase?: (update: AgentPhase) => void;
}

function runAgentLoop(params: AgentLoopParams) {
  const {
    t, goal, queryFn, term, getSubmitCommand,
    setPreview, setStreamText, streamingRef, executionModeRef,
    writeRed, abortRef, stepCount, maxSteps, history,
    onComplete, onFail,
  } = params;

  if (abortRef.current) return;
  if (stepCount >= maxSteps) {
    onFail(t.term_agent_max_steps(maxSteps));
    return;
  }

  // Build the query for the AI
  const webSearchNote = `\n\nNote: If you need to search the web for information, respond with command set to "AITERM_WEB_SEARCH: <your query>". If you need to fetch a specific URL, respond with command set to "AITERM_WEB_FETCH: <url>".`;
  let query: string;
  if (history.length === 0) {
    query = goal + `\n\nYou have access to web search. If you need internet information, respond with command set to "AITERM_WEB_SEARCH: <your query>" instead of a shell command.`;
  } else {
    query = `Goal: ${goal}\n\nExecution History:\n${history.map((h, i) =>
      `Step ${i + 1}:\nCommand: ${h.command}\nExit code: ${h.exitCode}\nOutput:\n${h.output}`
    ).join('\n\n')}\n\nAnalyze the result above and decide the next step to achieve the goal. If the goal is fully achieved, respond with command set to 'DONE'.${webSearchNote}`;
  }

  params.onPhase?.({ phase: "asking", step: stepCount + 1, maxSteps });

  // This callback fires when the command FINISHES executing in the PTY (via OSC 133;D)
  let stepResolved = false; // Set to true when either block completes OR AI returns DONE
  const onBlockDone = (completedBlock: TerminalBlock) => {
    stepResolved = true;
    if (abortRef.current) return;

    // Extract terminal output for this block
    let rawOutput = completedBlock.rawOutput.trim();
    if (rawOutput.length > 2000) rawOutput = rawOutput.slice(rawOutput.length - 2000);

    const exitCode = completedBlock.exitCode ?? 0;

    // Mirror this step (command + output) to the caller if it wired up onStepComplete.
    params.onStepComplete?.({
      stepIndex: stepCount + 1,
      maxSteps,
      command: completedBlock.command,
      exitCode,
      output: rawOutput,
    });

    const newHistory = [...history, {
      command: completedBlock.command,
      exitCode,
      output: rawOutput,
    }];

    // Recurse to next step
    runAgentLoop({
      ...params,
      history: newHistory,
      stepCount: stepCount + 1,
    });
  };

  // Wrap onComplete so we mark the step as resolved (prevents timeout from firing)
  const wrappedOnComplete = (explanation?: string) => {
    stepResolved = true;
    onComplete(explanation);
  };

  // Timeout: if the command hasn't completed in 60s, it likely needs user input
  setTimeout(() => {
    if (!stepResolved && !abortRef.current) {
      term.write(`\r\n\x1b[33m${t.term_agent_timeout}\x1b[0m\r\n`);
      onFail(t.term_agent_timeout_fail);
    }
  }, 60000);

  // Handle web search/fetch actions from the AI (intercept before PTY execution)
  const onWebAction = (type: "search" | "fetch", value: string) => {
    stepResolved = true; // prevent timeout from firing while waiting for web result
    params.onPhase?.({ phase: "web", step: stepCount + 1, maxSteps, query: value, webKind: type });
    const webPromise = type === "search" ? webSearch(value) : webFetch(value);
    webPromise
      .then((result) => {
        if (abortRef.current) return;
        stepResolved = false; // reset so next step timeout works
        const syntheticCommand = type === "search" ? `web_search("${value}")` : `web_fetch("${value}")`;
        const newHistory = [...history, {
          command: syntheticCommand,
          exitCode: 0,
          output: result,
        }];
        params.onStepComplete?.({
          stepIndex: stepCount + 1,
          maxSteps,
          command: syntheticCommand,
          exitCode: 0,
          output: result.length > 2000 ? result.slice(result.length - 2000) : result,
        });
        runAgentLoop({
          ...params,
          history: newHistory,
          stepCount: stepCount + 1,
        });
      })
      .catch((err) => {
        if (abortRef.current) return;
        onFail(`Web ${type} failed: ${String(err)}`);
      });
  };

  // Call AI, auto-execute the returned command, wire up the completion callback
  handleAiQuery(
    t,
    queryFn,
    "",
    query,
    term,
    setPreview,
    setStreamText,
    streamingRef,
    executionModeRef,
    writeRed,
    getSubmitCommand(),  // always get the LATEST submitCommand
    wrappedOnComplete,   // onDone: AI returned "DONE" → mark resolved & complete
    true,                // agentActive: force auto-execute for safe commands
    onBlockDone,         // onCommandComplete: fires when OSC 133;D marks the block done
    (err) => {           // onAiError: AI call failed, abort the mission immediately
      stepResolved = true;
      onFail(formatAiError(err));
    },
    onWebAction,          // onWebAction: intercept web search/fetch commands
    params.onPhase,       // onPhase: push running-phase status to the React status bar
    stepCount + 1,        // agentStep (1-based)
    maxSteps,             // agentMaxSteps
  );
}

/**
 * Tauri may deliver `AiError` either as the serialized object directly or
 * wrapped in an `Error` whose message is the JSON. Coerce both forms.
 */
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

export {
  INITIAL_PREVIEW,
  shouldAutoExecute,
  handleAiQuery,
  runAgentLoop,
  normalizeAiError,
};
export type { PreviewState, AgentLoopParams };
