import { agentChat, type AgentToolDefinition, type ChatMessage } from "../ipc/ai";
import { readFile, writeTextFile, listDirectory, getSessionCwd } from "../ipc/fs";
import { writePty, getPtyRecentOutput } from "../ipc/pty";

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
    description: "Execute a shell command in the terminal and return its output.",
    input_schema: {
      type: "object",
      properties: { command: { type: "string", description: "Shell command to execute" } },
      required: ["command"],
    },
  },
};

export async function runSubAgent(
  sessionId: string,
  agent: AgentDefinition,
  task: string,
  onAction?: (action: SubAgentAction) => void,
  maxInnerIterations = 30,
  sharedContext = "",
  projectDir?: string,
): Promise<SubAgentResult> {
    const actions: SubAgentAction[] = [];
    const cwd = projectDir
      || await getSessionCwd(sessionId).catch(() => "(unknown)")
      || "(unknown)";

    const contextSection = sharedContext
      ? `\n## 先前迭代的累積 Context\n以下是整個任務到目前為止已完成與尚未完成的紀錄，請避免重複已完成的工作：\n${sharedContext}\n`
      : "";

    const systemPrompt = `${agent.roleDescription}

Current working directory: ${cwd}
All file operations and shell commands MUST be performed under this directory. Use absolute paths when writing files.
${contextSection}
## 指示
使用可用的工具完成指派的任務。完成後，用繁體中文回報：
1. 你做了什麼（具體行動）
2. 結果是什麼（具體產出或發現）
3. 是否有任何問題或未完成的部分

不要重複 Context 中已標記為完成的工作。`;

    const enabledToolDefs = agent.tools.map(t => TOOL_DEFS[t]);

    const history: ChatMessage[] = [
      { role: "system", content: systemPrompt },
      { role: "user", content: task },
    ];

    const limit = maxInnerIterations === 0 ? Infinity : maxInnerIterations;
    for (let i = 0; i < limit; i++) {
      const reply = await agentChat(agent.providerId, history, enabledToolDefs, sessionId);

      if (reply.tool_calls.length === 0) {
        // Final answer — no more tool calls
        return { answer: reply.content ?? "", actions, isError: false };
      }

      // Execute each tool call
      for (const tc of reply.tool_calls) {
        if (!agent.tools.includes(tc.tool_name as AgentToolName)) {
          const errMsg = `Tool '${tc.tool_name}' is not enabled for agent '${agent.name}'`;
          const action: SubAgentAction = { tool: tc.tool_name, input: JSON.stringify(tc.args), output: errMsg, isError: true };
          actions.push(action);
          onAction?.(action);
          history.push({ role: "assistant", content: JSON.stringify(tc.args) });
          history.push({ role: "user", content: `Tool result for ${tc.tool_name}:\n${errMsg}` });
          continue;
        }

        let result: string;
        let isError = false;
        const args = tc.args as Record<string, string>;

        try {
          switch (tc.tool_name) {
            case "read_file": {
              const { content, truncated } = await readFile(args.path);
              result = truncated ? `${content}\n[...truncated]` : content;
              break;
            }
            case "write_file": {
              await writeTextFile(args.path, args.content);
              result = `Successfully wrote ${args.path}`;
              break;
            }
            case "list_directory": {
              const entries = await listDirectory(sessionId, args.path ?? "");
              result = entries.map(e => (e.is_dir ? `${e.name}/` : e.name)).join("\n") || "(empty)";
              break;
            }
            case "execute_command": {
              result = await executeCommandInPty(sessionId, args.command);
              break;
            }
            default:
              result = `Unknown tool: ${tc.tool_name}`;
              isError = true;
          }
        } catch (e) {
          result = `Error: ${serializeError(e)}`;
          isError = true;
        }

        const action: SubAgentAction = { tool: tc.tool_name, input: JSON.stringify(args), output: result, isError };
        actions.push(action);
        onAction?.(action);

        history.push({ role: "assistant", content: `<tool_call>${JSON.stringify({ name: tc.tool_name, arguments: args })}</tool_call>` });
        history.push({ role: "user", content: `Tool result for ${tc.tool_name}:\n${result}` });
      }
    }

  const limitMsg = maxInnerIterations === 0
    ? "Sub-agent exited loop unexpectedly."
    : `Sub-agent reached max tool iterations (${maxInnerIterations}) without completing the task.`;
  return { answer: limitMsg, actions, isError: true };
}

/** Write a command to PTY and poll until a sentinel marker appears in output. */
async function executeCommandInPty(sessionId: string, command: string): Promise<string> {
  const sentinel = `__AGENT_DONE_${Date.now()}__`;
  await writePty(sessionId, `${command}; echo "${sentinel}"\n`);

  const MAX_POLLS = 60;
  const POLL_INTERVAL = 1000;

  for (let i = 0; i < MAX_POLLS; i++) {
    await new Promise(r => setTimeout(r, POLL_INTERVAL));
    const output = await getPtyRecentOutput(sessionId).catch(() => null) ?? "";
    if (output.includes(sentinel)) {
      const beforeSentinel = output.split(sentinel)[0] ?? "";
      const lines = beforeSentinel.split("\n");
      // Drop the command line itself (first line) and trim
      const result = lines.slice(1).join("\n").trim();
      return result.slice(-4000); // cap at 4000 chars
    }
  }

  // Timeout — return whatever output we have
  const output = await getPtyRecentOutput(sessionId).catch(() => null) ?? "";
  return `[timeout after 60s]\n${output.slice(-2000)}`;
}
