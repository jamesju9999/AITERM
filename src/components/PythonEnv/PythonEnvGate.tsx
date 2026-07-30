import { useEffect, useRef } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import type { InstallLogLine } from "../Settings/McpInstallTerminal";
import type { GateState } from "./usePythonEnvGate";
import "./PythonEnvGate.css";

interface Props {
  state: GateState;
  lines: InstallLogLine[];
  error?: string;
  onInstall: () => void;
  onRecheck: () => void;
  onPickInterpreter?: () => void;
  onDismiss?: () => void;
}

export function PythonEnvGate({ state, lines, error, onInstall, onRecheck, onPickInterpreter, onDismiss }: Props) {
  const { t } = useLocale();
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [lines]);

  if (state === "ready") return null;

  // The install log and any captured error are meaningful in every non-ready
  // state, not just "installing" — a failure that flips the state to
  // missing/failed/broken must not make the log (or the error) disappear,
  // since that log is often the only place the real uv/pip error is visible.
  return (
    <div className="python-env-gate">
      {/* No dismiss while installing — closing the card mid-install could
          read as "cancel the install", which it doesn't do. */}
      {onDismiss && state !== "installing" && (
        <button
          className="python-env-gate__dismiss"
          onClick={onDismiss}
          aria-label={t.python_env_dismiss}
        >
          ×
        </button>
      )}

      {state === "installing" && (
        <>
          <div className="python-env-gate__title">{t.python_env_preparing}</div>
          <div className="python-env-gate__hint">{t.python_env_first_run_hint}</div>
        </>
      )}

      {state === "missing" && (
        <>
          <div className="python-env-gate__title">{t.python_env_missing_title}</div>
          <div className="python-env-gate__hint">{t.python_env_missing_body}</div>
        </>
      )}

      {state === "failed" && (
        <div className="python-env-gate__title">{t.python_env_failed_title}</div>
      )}

      {state === "broken" && (
        <div className="python-env-gate__title">{t.python_env_broken_title}</div>
      )}

      {error && <div className="python-env-gate__error">{error}</div>}

      {lines.length > 0 && (
        <div className="python-env-gate__log">
          {lines.map((line, i) => (
            <div
              key={i}
              className={`python-env-gate__log-line python-env-gate__log-line--${
                line.isError ? "error" : "ok"
              }`}
            >
              {line.text}
            </div>
          ))}
          <div ref={bottomRef} />
        </div>
      )}

      {state === "missing" && (
        <>
          <div className="python-env-gate__actions">
            <button className="python-env-gate__btn python-env-gate__btn--primary" onClick={onInstall}>
              {t.python_env_install}
            </button>
            <button className="python-env-gate__btn" onClick={onRecheck}>
              {t.python_env_recheck}
            </button>
            {onPickInterpreter && (
              <button className="python-env-gate__btn" onClick={onPickInterpreter}>
                {t.python_env_pick_interpreter}
              </button>
            )}
          </div>
          <div className="python-env-gate__manual-hint">{t.python_env_manual_hint}</div>
        </>
      )}

      {state === "failed" && (
        <div className="python-env-gate__actions">
          <button className="python-env-gate__btn python-env-gate__btn--primary" onClick={onInstall}>
            {t.python_env_retry}
          </button>
        </div>
      )}

      {/* "broken" gets no retry button — the bundled uv binary is what's
          broken, and clicking "install" would just fail the same way again.
          Reinstalling AITerm is the actual fix. */}
    </div>
  );
}
