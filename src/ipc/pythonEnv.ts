import { invoke } from "@tauri-apps/api/core";

export type PythonProfile = "api_docs" | "doc_core" | "doc_media";

export interface PythonEnvStatus {
  uvAvailable: boolean;
  pythonVersion: string | null;
  installed: PythonProfile[];
  venvPath: string;
  userInterpreter: string | null;
}

export interface PythonEnvLogEvent {
  level: "info" | "warn" | "error";
  message: string;
}

export const pythonEnvStatus = (): Promise<PythonEnvStatus> =>
  invoke("python_env_status");

export const pythonEnvEnsure = (profile: PythonProfile): Promise<void> =>
  invoke("python_env_ensure", { profile });

/** `purgeRuntimes` also deletes downloaded interpreters, not just the venv. */
export const pythonEnvReset = (purgeRuntimes: boolean): Promise<void> =>
  invoke("python_env_reset", { purgeRuntimes });

export const pythonEnvSetInterpreter = (path: string | null): Promise<void> =>
  invoke("python_env_set_interpreter", { path });
