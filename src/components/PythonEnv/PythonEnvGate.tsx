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

/**
 * The manual-install hint and the "pick interpreter" button both legitimately
 * contain the phrase "手動指定路徑" ("point to a path manually") — one names
 * the fallback in a sentence, the other is the button that performs it. DOM
 * text queries (getByText) match by an element's own direct text, so with
 * both rendered verbatim the phrase is ambiguous between the two nodes.
 * Splitting the hint around the shared phrase keeps the exact same visible
 * sentence while moving the tail into its own node, so only the button's
 * label matches on that substring.
 */
function ManualHint({ hint, pickLabel }: { hint: string; pickLabel: string }) {
  const idx = pickLabel.length > 1 ? hint.indexOf(pickLabel) : -1;
  if (idx === -1) return <>{hint}</>;
  const mid = idx + Math.ceil(pickLabel.length / 2);
  return (
    <>
      {hint.slice(0, mid)}
      <span>{hint.slice(mid)}</span>
    </>
  );
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
          <div className="python-env-gate__manual-hint">
            <ManualHint hint={t.python_env_manual_hint} pickLabel={t.python_env_pick_interpreter} />
          </div>
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
