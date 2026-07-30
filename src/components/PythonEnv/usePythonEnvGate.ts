import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  pythonEnvEnsure,
  pythonEnvStatus,
  type PythonEnvLogEvent,
  type PythonProfile,
} from "../../ipc/pythonEnv";
import type { InstallLogLine } from "../Settings/McpInstallTerminal";

export type GateState = "ready" | "installing" | "missing" | "failed";

/**
 * Shared state for every feature that needs the managed Python environment:
 * document conversion, API doc scraping, and knowledge-base import all use this
 * rather than each wiring up its own listener and state machine.
 */
export function usePythonEnvGate() {
  const [state, setState] = useState<GateState>("ready");
  const [lines, setLines] = useState<InstallLogLine[]>([]);
  const [error, setError] = useState<string>();
  // Tauri's listen resolves after unmount in tests and on fast navigation.
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    const unlisten = listen<PythonEnvLogEvent>("python-env-log", (event) => {
      if (!mounted.current) return;
      setLines((prev) => [
        ...prev,
        { text: event.payload.message, isError: event.payload.level !== "info" },
      ]);
    });
    return () => {
      mounted.current = false;
      unlisten.then((off) => off());
    };
  }, []);

  /**
   * Prepare `profile`, returning false when the caller should stop and let the
   * gate's UI take over.
   *
   * Which UI that is comes from `pythonEnvStatus()`, not from reading words out
   * of the error string — this app has been burned once by inferring a cause
   * from a message (a fake rate_limit_error rendered as "too many requests",
   * which sent debugging in the wrong direction for a day).
   */
  const ensureProfile = useCallback(async (profile: PythonProfile): Promise<boolean> => {
    setLines([]);
    setError(undefined);
    setState("installing");
    try {
      await pythonEnvEnsure(profile);
      setState("ready");
      return true;
    } catch (e) {
      setError(String(e));
      const status = await pythonEnvStatus().catch(() => null);
      setState(status && status.pythonVersion === null ? "missing" : "failed");
      return false;
    }
  }, []);

  const dismiss = useCallback(() => setState("ready"), []);

  return { state, lines, error, ensureProfile, dismiss };
}
