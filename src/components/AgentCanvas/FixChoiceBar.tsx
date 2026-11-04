import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { FixChoiceItem, FixMode } from "../../ipc/tasks";

/**
 * 互動卡被審核者退回後，修正卡先擋住等使用者選：新開一張自動修正卡，
 * 或接回原本那段對話自己改。一個模組一列；選擇由呼叫端送去後端。
 */
export function FixChoiceBar({
  items,
  onChoose,
}: {
  items: FixChoiceItem[];
  onChoose: (moduleId: string, mode: FixMode, remember: boolean) => void | Promise<void>;
}) {
  const { t } = useLocale();
  // 「之後都這樣」每個模組各自一格；預設勾選（沒動過的模組不在表裡）。
  const [keep, setKeep] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<{ moduleId: string; text: string } | null>(null);

  if (items.length === 0) return null;

  const choose = async (moduleId: string, mode: FixMode) => {
    setBusy(moduleId);
    setError(null);
    try {
      await onChoose(moduleId, mode, keep[moduleId] ?? true);
    } catch (e) {
      setError({ moduleId, text: String(e) });
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="ac-fixchoice" data-testid="fixchoice-bar" role="group">
      {items.map((it) => (
        <div className="ac-fixchoice-row" key={it.module_id}>
          <div className="ac-fixchoice-title">{t.ac_fixchoice_title(it.module_title, it.round)}</div>
          <div className="ac-fixchoice-actions">
            <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" data-testid="fixchoice-fresh" disabled={busy === it.module_id} onClick={() => void choose(it.module_id, "fresh")}>
              {t.ac_fixchoice_fresh}
            </button>
            <button
              type="button"
              className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
              data-testid="fixchoice-resume"
              disabled={!it.can_resume || busy === it.module_id}
              title={it.can_resume ? undefined : t.ac_fixchoice_resume_gone}
              onClick={() => void choose(it.module_id, "resume")}
            >
              {t.ac_fixchoice_resume}
            </button>
            <label className="ac-fixchoice-remember" title={t.ac_fixchoice_remember_hint}>
              <input
                type="checkbox"
                data-testid="fixchoice-remember"
                checked={keep[it.module_id] ?? true}
                onChange={(e) => setKeep((cur) => ({ ...cur, [it.module_id]: e.target.checked }))}
              />
              {t.ac_fixchoice_remember}
            </label>
          </div>
          <div className="ac-fixchoice-note">
            {t.ac_fixchoice_hold} {t.ac_fixchoice_remember_hint}
            {it.remembered && <span className="ac-fixchoice-last"> {t.ac_fixchoice_last(it.remembered)}</span>}
          </div>
          {error?.moduleId === it.module_id && (
            <div className="ac-fixchoice-error" role="alert">
              {error.text}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
