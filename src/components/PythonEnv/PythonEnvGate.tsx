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
}

export function PythonEnvGate({ state, lines, error, onInstall, onRecheck, onPickInterpreter }: Props) {
  const { t } = useLocale();
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [lines]);

  if (state === "ready") return null;

  return (
    <div className="python-env-gate">
      {state === "installing" && (
        <>
          <div className="python-env-gate__title">{t.python_env_preparing}</div>
          <div className="python-env-gate__hint">{t.python_env_first_run_hint}</div>
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
        </>
      )}

      {state === "missing" && (
        <>
          <div className="python-env-gate__title">{t.python_env_missing_title}</div>
          <div className="python-env-gate__hint">{t.python_env_missing_body}</div>
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
        <>
          <div className="python-env-gate__title">{t.python_env_failed_title}</div>
          {error && <div className="python-env-gate__error">{error}</div>}
          <div className="python-env-gate__actions">
            <button className="python-env-gate__btn python-env-gate__btn--primary" onClick={onInstall}>
              {t.python_env_retry}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
