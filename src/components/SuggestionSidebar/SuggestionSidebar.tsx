import { useEffect, useRef, useState, type FormEvent } from "react";
import { abortAi, invokeAiChatCtx } from "../../ipc/ai";
import { languageDirective } from "../../lib/i18n";
import { describeError } from "./describeError";
import { useLocale } from "../../contexts/LocaleContext";
import { BUILTIN_AI_CLI_NAMES, normalizeCliName } from "../../lib/aiCliCommand";
import { MAX_GOAL_CHARS, buildGoalPolishRequest, cleanPolishedGoal } from "../../lib/promptSuggestions";
import { SparklesIcon } from "../Icons";
import { loadRedactEnabled, saveRedactEnabled } from "./redactSetting";

const CTX_OPEN_KEY = "aiterm-suggest-ctx-open";
/** 脈絡區（目標與里程碑）預設展開；使用者收起就記住。 */
function loadCtxOpen(): boolean {
  try { return localStorage.getItem(CTX_OPEN_KEY) !== "false"; } catch { return true; }
}
function saveCtxOpen(open: boolean): void {
  try { localStorage.setItem(CTX_OPEN_KEY, String(open)); } catch { /* ignore */ }
}
import { MilestoneList } from "./MilestoneList";
import { PromptAssistant } from "./PromptAssistant";
import { PromptSuggestions } from "./PromptSuggestions";
import { formatMilestonesForPrompt, resolveFocus, type MilestoneState } from "../../lib/milestones";
import "./SuggestionSidebar.css";

export interface SuggestionSidebarProps {
  sessionId: string;
  providerId?: string;
  /** Agent 執行中之類由外層決定的暫停條件。 */
  disabled: boolean;
  getIdleMs?: () => number;
  /** AI 工具先前每一輪的穩定畫面（舊→新），給建議當「已經做過什麼」的依據。 */
  getHistory?: () => string[];
  /** 終端機裡現在是否有 AI 命令列工具在執行（只影響狀態文字）。 */
  aiCliRunning: boolean;
  customNames: readonly string[];
  onCustomNamesChange: (names: string[]) => void;
  onClose: () => void;
  /** 這個分頁的大目標（沒有就是空字串）。 */
  goal: string;
  onGoalChange: (goal: string) => void;
  /** 這個分頁的里程碑（沒有就是 undefined）。 */
  milestones: MilestoneState | undefined;
  onMilestonesChange: (next: MilestoneState | undefined) => void;
}

export function SuggestionSidebar({
  sessionId, providerId, disabled, getIdleMs, getHistory, aiCliRunning, customNames, onCustomNamesChange, onClose, goal, onGoalChange, milestones, onMilestonesChange,
}: SuggestionSidebarProps) {
  const { t, locale } = useLocale();
  const [menuOpen, setMenuOpen] = useState(false);
  const [redact, setRedact] = useState(loadRedactEnabled);
  const [ctxOpen, setCtxOpen] = useState(loadCtxOpen);
  const toggleCtx = () => { const next = !ctxOpen; setCtxOpen(next); saveCtxOpen(next); };
  const toggleRedact = () => { const next = !redact; setRedact(next); saveRedactEnabled(next); };
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [editingGoal, setEditingGoal] = useState(false);
  /** 使用者選的焦點里程碑；只在記憶體，實際生效的焦點由 resolveFocus 決定。 */
  const [chosenFocusId, setChosenFocusId] = useState<string | null>(null);
  const [goalDraft, setGoalDraft] = useState("");

  const [polishing, setPolishing] = useState(false);
  const [polishError, setPolishError] = useState<string | null>(null);
  /** 潤飾前使用者自己寫的原稿；有值才顯示「還原」。使用者再動手改字就清掉。 */
  const [undoText, setUndoText] = useState<string | null>(null);
  const polishReqRef = useRef(0);
  const polishingRef = useRef(false);
  const polishConnId = `goal-polish-${sessionId}`;

  /** 讓還在路上的潤飾回應過期並取消請求。 */
  const cancelPolish = () => {
    polishReqRef.current += 1;
    if (polishingRef.current) { polishingRef.current = false; abortAi(polishConnId).catch(() => {}); }
    setPolishing(false);
  };
  useEffect(() => () => {
    if (polishingRef.current) abortAi(polishConnId).catch(() => {});
  }, [polishConnId]);

  const closeGoalEditor = () => {
    cancelPolish();
    setPolishError(null);
    setUndoText(null);
    setEditingGoal(false);
  };
  const milestoneItems = milestones?.items ?? [];
  const focusId = resolveFocus(milestoneItems, chosenFocusId);
  const focusText = milestoneItems.find((i) => i.id === focusId)?.text ?? null;
  const allDone = milestoneItems.length > 0 && focusId === null;
  const milestoneContext = formatMilestonesForPrompt(milestoneItems, focusId);

  const startEditGoal = () => { setGoalDraft(goal); setPolishError(null); setUndoText(null); setEditingGoal(true); };
  const saveGoal = (e: FormEvent) => {
    e.preventDefault();
    onGoalChange(goalDraft.trim());
    closeGoalEditor();
  };
  const clearGoal = () => { onGoalChange(""); closeGoalEditor(); };

  const polishGoal = async () => {
    const original = goalDraft;
    if (!original.trim() || polishingRef.current) return;
    const myReq = ++polishReqRef.current;
    polishingRef.current = true;
    setPolishing(true);
    setPolishError(null);
    try {
      const reply = await invokeAiChatCtx(
        [{ role: "user", content: buildGoalPolishRequest(original, languageDirective(locale)) }],
        { os: navigator.platform.toLowerCase(), shell: null, cwd: null, recentOutput: null },
        polishConnId,
        providerId,
        locale,
      );
      if (myReq !== polishReqRef.current) return;
      const polished = cleanPolishedGoal(reply.content);
      if (!polished) { setPolishError(t.sugg_goal_polish_empty); return; }
      setUndoText(original);
      setGoalDraft(polished);
    } catch (e) {
      if (myReq !== polishReqRef.current) return;
      setPolishError(t.sugg_goal_polish_error(describeError(e)));
    } finally {
      if (myReq === polishReqRef.current) { polishingRef.current = false; setPolishing(false); }
    }
  };
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
          <label className="aiterm-sugg-sidebar__redact">
            <input type="checkbox" checked={redact} onChange={toggleRedact} />
            <span>{t.redact_toggle}</span>
          </label>
          <p className="aiterm-sugg-sidebar__help">{t.redact_toggle_hint}</p>
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

      <div className="aiterm-sugg-ctx">
        <button
          type="button"
          className="aiterm-sugg-ctx__bar"
          aria-expanded={ctxOpen}
          onClick={toggleCtx}
        >
          <span className={`aiterm-sugg-ctx__chevron${ctxOpen ? " aiterm-sugg-ctx__chevron--open" : ""}`} aria-hidden="true">▸</span>
          <span className="aiterm-sugg-ctx__lines">
            <span className="aiterm-sugg-ctx__line">{goal.trim() ? t.sugg_ctx_goal(goal.trim()) : t.sugg_ctx_none}</span>
            {milestoneItems.length > 0 && (
              <span className="aiterm-sugg-ctx__line aiterm-sugg-ctx__line--ms">
                {focusText === null
                  ? t.sugg_ctx_ms_done(milestoneItems.filter((i) => i.done).length, milestoneItems.length)
                  : t.sugg_ctx_ms(milestoneItems.filter((i) => i.done).length, milestoneItems.length, focusText)}
              </span>
            )}
          </span>
        </button>
        {/* 收起時只是藏起來、不卸載：寫到一半的目標、進行中的 AI 拆解都不會因為收合而丟掉。 */}
        <div className="aiterm-sugg-ctx__panel" hidden={!ctxOpen}>
        <section className="aiterm-sugg-goal">
          {editingGoal ? (
            <form onSubmit={saveGoal} className="aiterm-sugg-goal__form">
              <textarea
                aria-label={t.sugg_goal_label}
                value={goalDraft}
                onChange={(e) => { setGoalDraft(e.target.value); setUndoText(null); }}
                maxLength={MAX_GOAL_CHARS}
                rows={3}
                placeholder={t.sugg_goal_placeholder}
                autoFocus
              />
              <div className="aiterm-sugg-goal__polish">
                <button
                  type="button"
                  className="aiterm-sugg-goal__polish-btn"
                  disabled={!goalDraft.trim() || polishing}
                  title={t.sugg_goal_polish_title}
                  onClick={() => void polishGoal()}
                >
                  <SparklesIcon size={12} />
                  <span>{polishing ? t.sugg_goal_polishing : t.sugg_goal_polish}</span>
                </button>
                {undoText !== null && !polishing && (
                  <button type="button" className="aiterm-sugg-goal__undo" onClick={() => { setGoalDraft(undoText); setUndoText(null); }}>
                    {t.sugg_goal_undo}
                  </button>
                )}
              </div>
              {polishError && <div className="aiterm-sugg-sidebar__error" role="alert">{polishError}</div>}
              <span className="aiterm-sugg-goal__hint">{t.sugg_goal_hint}</span>
              <div className="aiterm-sugg-goal__actions">
                <button type="submit" className="aiterm-sugg-goal__save">{t.sugg_goal_save}</button>
                <button type="button" onClick={closeGoalEditor}>{t.sugg_goal_cancel}</button>
                {goal && <button type="button" className="aiterm-sugg-goal__clear" onClick={clearGoal}>{t.sugg_goal_clear}</button>}
              </div>
            </form>
          ) : goal ? (
            <div className="aiterm-sugg-goal__view">
              <span className="aiterm-sugg-goal__label">{t.sugg_goal_label}</span>
              <p className="aiterm-sugg-goal__text">{goal}</p>
              <button type="button" className="aiterm-sugg-sidebar__icon" aria-label={t.sugg_goal_edit} title={t.sugg_goal_edit} onClick={startEditGoal}>✎</button>
            </div>
          ) : (
            <button type="button" className="aiterm-sugg-goal__set" onClick={startEditGoal}>
              <span aria-hidden="true">＋</span>
              <span>{t.sugg_goal_set}</span>
            </button>
          )}
        </section>

        {(goal.trim() || milestones) && (
          <MilestoneList
            sessionId={sessionId}
            providerId={providerId}
            goal={goal}
            state={milestones}
            onChange={onMilestonesChange}
            getHistory={getHistory}
            focusId={focusId}
            onFocus={setChosenFocusId}
            redact={redact}
          />
        )}
        </div>
      </div>

      <div className="aiterm-sugg-sidebar__body">
        <PromptSuggestions
          sessionId={sessionId}
          providerId={providerId}
          disabled={disabled}
          getIdleMs={getIdleMs}
          hideTitle
          goal={goal}
          getHistory={getHistory}
          aiCliRunning={aiCliRunning}
          redact={redact}
          milestoneContext={milestoneContext}
          focusLabel={focusText}
          allMilestonesDone={allDone}
        />
      </div>

      <PromptAssistant
        sessionId={sessionId}
        providerId={providerId}
        goal={goal}
        milestoneContext={milestoneContext}
        redact={redact}
      />
    </aside>
  );
}
