import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { useIntakeRemote } from "../../hooks/useIntakeRemote";
import { getAskNotifyEmail, setAskNotifyEmail } from "../../ipc/serverQuestions";

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

/** 共用 server 的「Agent 提問通知信箱」（管理員限定）：連線中且是管理員才顯示。 */
export function AskNotifySettings() {
  const { t } = useLocale();
  const remote = useIntakeRemote();
  const enabled = remote.connected && remote.role === "admin";
  const [value, setValue] = useState("");
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    getAskNotifyEmail()
      .then((v) => {
        if (!alive) return;
        setValue(v);
        setSaved(v);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [enabled]);

  if (!enabled) return null;
  const save = async () => {
    setError(null);
    try {
      await setAskNotifyEmail(value.trim());
      setSaved(value.trim());
    } catch (e) {
      setError(errText(e));
    }
  };
  return (
    <div className="shared-server-ask" data-testid="ask-notify-settings">
      <label>
        <strong>{t.ac_ask_email}</strong>
        <input className="aiterm-input" type="email" value={value} onChange={(e) => setValue(e.target.value)} placeholder="name@example.com" />
      </label>
      <p className="shared-server-intro">{t.ac_ask_email_hint}</p>
      <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={value.trim() === (saved ?? "")} onClick={() => void save()}>
        {t.ac_ask_send}
      </button>
      {error && (
        <p className="ac-bad-text" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
