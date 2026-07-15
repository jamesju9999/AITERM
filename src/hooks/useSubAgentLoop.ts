import { agentChat, type AgentToolDefinition, type ChatMessage } from "../ipc/ai";
import { readFile, writeTextFile, listDirectory, getSessionCwd } from "../ipc/fs";
import { agentExec } from "../ipc/exec";
import { classifyCommand, commandWritesOutsideRoot } from "../lib/commandRisk";
import { isPathInside } from "../lib/pathUtils";
import { languageDirective, type Locale } from "../lib/i18n";

export function serializeError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  try { return JSON.stringify(e); } catch { return String(e); }
}

export interface AgentDefinition {
  name: string;
  providerId: string;
  roleDescription: string;
  tools: AgentToolName[];
}

export type AgentToolName = "read_file" | "write_file" | "list_directory" | "execute_command";

export interface SubAgentAction {
  tool: string;
  input: string;
  output: string;
  isError: boolean;
}

export interface SubAgentResult {
  answer: string;
  actions: SubAgentAction[];
  isError: boolean;
}

/** Context shared by all tool executions in one loop run. */
export interface ToolExecutionContext {
  sessionId: string;
  /** projectDir ?? session CWD — write_file confinement root and execute_command cwd. Null disables both. */
  effectiveRoot: string | null;
  /** Called before running a dangerous command. Resolve false to deny. Absent = deny dangerous commands. */
  onConfirmNeeded?: (command: string) => Promise<boolean>;
}

const TOOL_DEFS: Record<AgentToolName, AgentToolDefinition> = {
  read_file: {
    name: "read_file",
    description: "Read the contents of a text file at the given absolute path.",
    input_schema: {
      type: "object",
      properties: { path: { type: "string", description: "Absolute file path to read" } },
      required: ["path"],
    },
  },
  write_file: {
    name: "write_file",
    description: "Write or overwrite a text file at the given absolute path.",
    input_schema: {
      type: "object",
      properties: {
        path: { type: "string", description: "Absolute file path to write" },
        content: { type: "string", description: "Content to write" },
      },
      required: ["path", "content"],
    },
  },
  list_directory: {
    name: "list_directory",
    description: "List entries in a directory. Pass an empty string to list the current working directory.",
    input_schema: {
      type: "object",
      properties: { path: { type: "string", description: "Directory path, or empty string for CWD" } },
      required: ["path"],
    },
  },
  execute_command: {
    name: "execute_command",
    description: "Execute a shell command and return its output.",
    input_schema: {
      type: "object",
      properties: { command: { type: "string", description: "Shell command to execute" } },
      required: ["command"],
    },
  },
};

async function executeTool(
  name: AgentToolName,
  args: Record<string, string>,
  ctx: ToolExecutionContext,
): Promise<{ result: string; isError: boolean }> {
  try {
    switch (name) {
      case "read_file": {
        const { content, truncated } = await readFile(args.path);
        return { result: truncated ? `${content}\n[...truncated]` : content, isError: false };
      }
      case "write_file": {
        if (!ctx.effectiveRoot) {
          return {
            result: "Error: could not determine the project directory; refusing to write for safety. Please set a project directory in Loop Studio.",
            isError: true,
          };
        }
        if (!isPathInside(args.path, ctx.effectiveRoot)) {
          return {
            result: `Error: writing outside the project directory (${ctx.effectiveRoot}) is not allowed: ${args.path}. Please use a path inside the project directory instead.`,
            isError: true,
          };
        }
        await writeTextFile(args.path, args.content);
        return { result: `Successfully wrote ${args.path}`, isError: false };
      }
      case "list_directory": {
        const entries = await listDirectory(ctx.sessionId, args.path ?? "");
        return {
          result: entries.map(e => (e.is_dir ? `${e.name}/` : e.name)).join("\n") || "(empty)",
          isError: false,
        };
      }
      case "execute_command": {
        if (!ctx.effectiveRoot) {
          return {
            result: "Error: could not determine the project directory; refusing to execute commands for safety. Please set a project directory in Loop Studio.",
            isError: true,
          };
        }
        const root = ctx.effectiveRoot;
        const isDangerous = classifyCommand(args.command) === "dangerous" || commandWritesOutsideRoot(args.command, root);
        if (isDangerous) {
          if (!ctx.onConfirmNeeded) {
            return { result: "Error: this command was classified as dangerous, and dangerous commands are not allowed in this execution environment.", isError: true };
          }
          const approved = await ctx.onConfirmNeeded(args.command);
          if (!approved) {
            return { result: "The user declined to run this command. Please find another way to complete the task.", isError: true };
          }
        }
        const r = await agentExec(args.command, root);
        const combined = [r.stdout, r.stderr ? `[stderr]\n${r.stderr}` : ""].filter(Boolean).join("\n");
        if (r.timed_out) {
          return { result: `[timeout after 60s — process killed]\n${combined}`, isError: true };
        }
        if (r.exit_code !== 0) {
          return { result: `[exit code ${r.exit_code}]\n${combined}`, isError: true };
        }
        return { result: combined || "(no output)", isError: false };
      }
    }
  } catch (e) {
    return { result: `Error: ${serializeError(e)}`, isError: true };
  }
}

/**
 * Core tool-calling loop shared by sub-agents and the Verifier.
 * Mutates `history` in place using proper OpenAI tool-calling format:
 * one assistant message carrying tool_calls, then one `tool` message per call.
 */
export async function runToolLoop(
  providerId: string,
  history: ChatMessage[],
  enabledTools: AgentToolName[],
  ctx: ToolExecutionContext,
  maxIterations: number,
  onAction?: (action: SubAgentAction) => void,
): Promise<SubAgentResult> {
  const actions: SubAgentAction[] = [];
  const toolDefs = enabledTools.map(t => TOOL_DEFS[t]);
  const limit = maxIterations === 0 ? Infinity : maxIterations;

  for (let i = 0; i < limit; i++) {
    const reply = await agentChat(providerId, history, toolDefs, ctx.sessionId);

    if (reply.tool_calls.length === 0) {
      return { answer: reply.content ?? "", actions, isError: false };
    }

    // One assistant message with ALL tool_calls. Prefer raw_tool_calls
    // (preserves Gemini thought_signature); reconstruct otherwise.
    const assistantToolCalls =
      reply.raw_tool_calls ??
      reply.tool_calls.map((tc, idx) => ({
        id: tc.id || `call_${Date.now()}_${idx}`,
        type: "function" as const,
        function: { name: tc.tool_name, arguments: JSON.stringify(tc.args) },
      }));
    history.push({ role: "assistant", content: null, tool_calls: assistantToolCalls });

    for (let idx = 0; idx < reply.tool_calls.length; idx++) {
      const tc = reply.tool_calls[idx];
      const callId = assistantToolCalls[idx]?.id ?? tc.id ?? `call_${idx}`;

      let result: string;
      let isError: boolean;
      if (!enabledTools.includes(tc.tool_name as AgentToolName)) {
        result = `Tool '${tc.tool_name}' is not enabled for this agent`;
        isError = true;
      } else {
        ({ result, isError } = await executeTool(
          tc.tool_name as AgentToolName,
          tc.args as Record<string, string>,
          ctx,
        ));
      }

      const action: SubAgentAction = {
        tool: tc.tool_name,
        input: JSON.stringify(tc.args),
        output: result,
        isError,
      };
      actions.push(action);
      onAction?.(action);

      history.push({ role: "tool", content: result, tool_call_id: callId });
    }
  }

  const limitMsg = `Sub-agent reached max tool iterations (${maxIterations}) without completing the task.`;
  return { answer: limitMsg, actions, isError: true };
}

export interface RunSubAgentOptions {
  onAction?: (action: SubAgentAction) => void;
  onConfirmNeeded?: (command: string) => Promise<boolean>;
  maxInnerIterations?: number;
  sharedContext?: string;
  projectDir?: string;
  locale?: Locale;
}

export async function runSubAgent(
  sessionId: string,
  agent: AgentDefinition,
  task: string,
  options: RunSubAgentOptions = {},
): Promise<SubAgentResult> {
  const { onAction, onConfirmNeeded, maxInnerIterations = 30, sharedContext = "", projectDir, locale = "zh-TW" } = options;

  const cwd = projectDir
    || await getSessionCwd(sessionId).catch(() => null)
    || null;

  const contextSection = sharedContext
    ? `\n## Accumulated Context From Previous Iterations\nBelow is the record of what has and hasn't been completed for this task so far — avoid repeating completed work:\n${sharedContext}\n`
    : "";

  const systemPrompt = `${agent.roleDescription}

Current working directory: ${cwd ?? "(unknown)"}
All file operations and shell commands MUST be performed under this directory. Use absolute paths when writing files.
${contextSection}
## Instructions
Use the available tools to complete the assigned task. When done, report in ${languageDirective(locale)}:
1. What you did (concrete actions)
2. What the result was (concrete output or findings)
3. Whether there were any issues or incomplete parts

Do not repeat work already marked as completed in the Context.`;

  const history: ChatMessage[] = [
    { role: "system", content: systemPrompt },
    { role: "user", content: task },
  ];

  return runToolLoop(
    agent.providerId,
    history,
    agent.tools,
    { sessionId, effectiveRoot: cwd, onConfirmNeeded },
    maxInnerIterations,
    onAction,
  );
}
