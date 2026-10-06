import { useState, type FormEvent } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { BUILTIN_AI_CLI_NAMES, normalizeCliName } from "../../lib/aiCliCommand";
import { SparklesIcon } from "../Icons";
import { PromptSuggestions } from "./PromptSuggestions";
import "./SuggestionSidebar.css";

export interface SuggestionSidebarProps {
  sessionId: string;
  providerId?: string;
  /** Agent 執行中之類由外層決定的暫停條件。 */
  disabled: boolean;
  getIdleMs?: () => number;
  /** 終端機裡現在是否有 AI 命令列工具在執行（只影響狀態文字）。 */
  aiCliRunning: boolean;
  customNames: readonly string[];
  onCustomNamesChange: (names: string[]) => void;
  onClose: () => void;
}

export function SuggestionSidebar({
  sessionId, providerId, disabled, getIdleMs, aiCliRunning, customNames, onCustomNamesChange, onClose,
}: SuggestionSidebarProps) {
  const { t } = useLocale();
  const [menuOpen, setMenuOpen] = useState(false);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);

  const addName = (e: FormEvent) => {
    e.preventDefault();
    const name = normalizeCliName(draft);
    if (!name) { setError(t.sugg_custom_invalid); return; }
    if (BUILTIN_AI_CLI_NAMES.includes(name) || customNames.includes(name)) { setError(t.sugg_custom_exists); return; }
    onCustomNamesChange([...customNames, name]);
    setDraft("");
    setError(null);
  };

  return (
    <aside className="aiterm-sugg-sidebar" aria-label={t.suggest_title}>
      <header className="aiterm-sugg-sidebar__head">
        <div className="aiterm-sugg-sidebar__heading">
          <span className="aiterm-sugg-sidebar__title"><SparklesIcon size={14} />{t.suggest_title}</span>
          <span className={`aiterm-sugg-sidebar__status${aiCliRunning ? " aiterm-sugg-sidebar__status--on" : ""}`}>
            {aiCliRunning ? t.sugg_status_detected : t.sugg_status_manual}
          </span>
        </div>
        <div className="aiterm-sugg-sidebar__actions">
          <button
            type="button"
            className="aiterm-sugg-sidebar__icon"
            aria-label={t.sugg_menu}
            aria-expanded={menuOpen}
            title={t.sugg_menu}
            onClick={() => setMenuOpen((o) => !o)}
          >⋯</button>
          <button
            type="button"
            className="aiterm-sugg-sidebar__icon"
            aria-label={t.sugg_close}
            title={t.sugg_close}
            onClick={onClose}
          >×</button>
        </div>
      </header>

      {menuOpen && (
        <section className="aiterm-sugg-sidebar__menu" aria-label={t.sugg_custom_title}>
          <p className="aiterm-sugg-sidebar__help">{t.sugg_custom_help}</p>
          <div className="aiterm-sugg-sidebar__group">
            <span className="aiterm-sugg-sidebar__group-label">{t.sugg_custom_builtin}</span>
            <ul className="aiterm-sugg-sidebar__chips">
              {BUILTIN_AI_CLI_NAMES.map((n) => <li key={n} className="aiterm-sugg-sidebar__chip">{n}</li>)}
            </ul>
          </div>
          <div className="aiterm-sugg-sidebar__group">
            <span className="aiterm-sugg-sidebar__group-label">{t.sugg_custom_mine}</span>
            {customNames.length === 0 ? (
              <span className="aiterm-sugg-sidebar__none">{t.sugg_custom_none}</span>
            ) : (
              <ul className="aiterm-sugg-sidebar__chips">
                {customNames.map((n) => (
                  <li key={n} className="aiterm-sugg-sidebar__chip aiterm-sugg-sidebar__chip--custom">
                    <span>{n}</span>
                    <button
                      type="button"
                      aria-label={t.sugg_custom_remove(n)}
                      onClick={() => onCustomNamesChange(customNames.filter((x) => x !== n))}
                    >×</button>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <form className="aiterm-sugg-sidebar__add" onSubmit={addName}>
            <input
              value={draft}
              onChange={(e) => { setDraft(e.target.value); setError(null); }}
              placeholder={t.sugg_custom_placeholder}
              spellCheck={false}
              autoCapitalize="off"
              autoCorrect="off"
            />
            <button type="submit">{t.sugg_custom_add}</button>
          </form>
          {error && <div className="aiterm-sugg-sidebar__error" role="alert">{error}</div>}
        </section>
      )}

      <div className="aiterm-sugg-sidebar__body">
        <PromptSuggestions
          sessionId={sessionId}
          providerId={providerId}
          disabled={disabled}
          getIdleMs={getIdleMs}
          hideTitle
        />
      </div>
    </aside>
  );
}
