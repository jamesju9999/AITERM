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
  const logRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    // Scroll the log element itself rather than calling scrollIntoView on a
    // sentinel inside it. This card sits in normal flow, unlike
    // McpInstallTerminal (position: fixed) whose pattern this borrowed —
    // scrollIntoView walks up to the nearest scrollable ancestor, which here is
    // the document, so every log line during an install scrolled the whole app
    // upward and left it there once the card unmounted.
    const el = logRef.current;
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
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
        <div className="python-env-gate__log" ref={logRef}>
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
        </div>
      )}

      {state === "missing" && (
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
      )}

      {state === "failed" && (
        <div className="python-env-gate__actions">
          <button className="python-env-gate__btn python-env-gate__btn--primary" onClick={onInstall}>
            {t.python_env_retry}
          </button>
          {onPickInterpreter && (
            <button className="python-env-gate__btn" onClick={onPickInterpreter}>
              {t.python_env_pick_interpreter}
            </button>
          )}
        </div>
      )}

      {/* Both escape hatches this hint describes (interpreter for a blocked
          GitHub, Index URL for a blocked PyPI) apply here. "missing" is uv
          failing to fetch Python; "failed" is Python and the venv working fine
          but `uv pip install` failing to reach PyPI — the actual path a
          PyPI-blocked user hits, since the venv gets built before pip runs. If
          this only rendered for "missing", the one user it exists for would
          never see it. */}
      {(state === "missing" || state === "failed") && (
        <div className="python-env-gate__manual-hint">{t.python_env_manual_hint}</div>
      )}

      {/* "broken" gets no retry button — the bundled uv binary is what's
          broken, and clicking "install" would just fail the same way again.
          Reinstalling AITerm is the actual fix. */}
    </div>
  );
}
