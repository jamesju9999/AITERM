import { useCallback, useEffect, useState } from "react";
import { intakeTabStart, intakeTabStop, intakeTabs } from "../../ipc/intake";
import { useIntakeChanged } from "../../hooks/useIntakeChanged";
import { useLocale } from "../../contexts/LocaleContext";
import { MailIcon } from "../Icons";
import "./index.css";

interface Props {
  /** **PTY session id，不是 React 的分頁 id**（後端靠它找到終端機）。 */
  sessionId: string;
}

/** 終端機工具列上的「啟動問題收集」切換鈕：這個分頁成為問題的接收端。 */
export function IntakeToggle({ sessionId }: Props) {
  const { t } = useLocale();
  const [on, setOn] = useState(false);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => {
    intakeTabs()
      .then((tabs) => setOn(tabs.includes(sessionId)))
      .catch(() => {});
  }, [sessionId]);

  useEffect(refresh, [refresh]);
  useIntakeChanged(refresh);

  async function toggle() {
    if (busy) return;
    setBusy(true);
    try {
      if (on) await intakeTabStop(sessionId);
      else await intakeTabStart(sessionId);
      refresh();
    } finally {
      setBusy(false);
    }
  }

  return (
    <button
      className={`aiterm-btn aiterm-btn--secondary aiterm-btn--sm ${on ? "aiterm-intake-toggle--on" : ""}`}
      title={t.intake_toggle_tooltip}
      aria-pressed={on}
      disabled={busy}
      onClick={(e) => {
        e.stopPropagation();
        void toggle();
      }}
      style={{ display: "flex", alignItems: "center", gap: "6px" }}
    >
      <MailIcon size={14} />
      <span>{on ? t.intake_toggle_on : t.intake_toggle_off}</span>
    </button>
  );
}
