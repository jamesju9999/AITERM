import { invoke } from "@tauri-apps/api/core";

export interface ExecResult {
  stdout: string;
  stderr: string;
  exit_code: number | null;
  timed_out: boolean;
}

/** Run a shell command via the backend (sh -c / cmd /C). Default timeout 60s. */
export const agentExec = (
  command: string,
  cwd?: string,
  timeoutMs?: number,
): Promise<ExecResult> =>
  invoke<ExecResult>("agent_exec", {
    command,
    cwd: cwd ?? null,
    timeoutMs: timeoutMs ?? null,
  });
