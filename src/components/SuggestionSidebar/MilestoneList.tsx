import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import { abortAi, invokeAiComplete } from "../../ipc/ai";
import { useLocale } from "../../contexts/LocaleContext";
import { languageDirective } from "../../lib/i18n";
import {
  MAX_MILESTONES,
  MAX_MILESTONE_CHARS,
  buildMilestoneCheckRequest,
  buildMilestonePlanRequest,
  mergePlanKeepingDone,
  newMilestone,
  parseMilestoneCheck,
  parseMilestonePlan,
  type Milestone,
  type MilestoneState,
} from "../../lib/milestones";
import { MAX_SCREEN_CHARS, PLAIN_COMPLETION_SYSTEM_PROMPT } from "../../lib/promptSuggestions";
import { PROMPT_HISTORY_BUDGET, formatHistoryForPrompt, normalizeScreen } from "../../lib/screenHistory";
import { redactSecrets } from "../../lib/redact";
import { serializeTerminal } from "../../lib/terminalInstanceRegistry";
import { describeError } from "./describeError";
import "./MilestoneList.css";

type Busy = null | "plan" | "check";

export interface MilestoneListProps {
  sessionId: string;
  providerId?: string;
  /** 目前的大目標。 */
  goal: string;
  /** 目前的里程碑（沒有就是 undefined）。 */
  state: MilestoneState | undefined;
  onChange: (next: MilestoneState | undefined) => void;
  /** AI 工具先前每一輪的穩定畫面，「檢查進度」用來判斷做過什麼。 */
  getHistory?: () => string[];
  /** 目前被建議朝向的焦點里程碑 id（沒有＝null）。 */
  focusId: string | null;
  /** 送出畫面給 AI 前先遮罩敏感資訊。 */
  redact: boolean;
  /** 使用者想把焦點移到另一個里程碑。焦點只是一種「現在想先做哪個」，不寫進里程碑資料。 */
  onFocus: (id: string) => void;
}

/**
 * 大目標拆出來的里程碑：可手動增刪改排序與勾選；也可請 AI 拆解、請 AI 檢查進度。
 *
 * **AI 只提議，不動資料**：拆解結果要使用者按「採用」才寫入，檢查進度也是逐項「採用」才打勾。
 * 自動勾選一旦誤判就會一路累積，而使用者看不到它改了什麼。
 */
export function MilestoneList({ sessionId, providerId, goal, state, onChange, getHistory, focusId, onFocus, redact }: MilestoneListProps) {
  const { t, locale } = useLocale();
  const items = state?.items ?? [];
  const done = items.filter((i) => i.done).length;

  const [collapsed, setCollapsed] = useState(false);
  /** null＝沒在編輯；"new"＝新增中；其他＝正在編輯那個 id。 */
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState<Busy>(null);
  const [error, setError] = useState<string | null>(null);
  const [planProposal, setPlanProposal] = useState<string[] | null>(null);
  const [checkProposal, setCheckProposal] = useState<{ ids: string[]; note: string } | null>(null);
  const [checkNone, setCheckNone] = useState(false);
  const [redactedCount, setRedactedCount] = useState(0);

  const reqRef = useRef(0);
  const busyRef = useRef<Busy>(null);
  const connId = (kind: "plan" | "check") => `milestone-${kind}-${sessionId}`;

  /** 讓還在路上的回應過期、取消請求、清掉所有暫時狀態。 */
  const cancelInFlight = () => {
    reqRef.current += 1;
    if (busyRef.current) abortAi(connId(busyRef.current)).catch(() => {});
    busyRef.current = null;
    setBusy(null);
  };
  const resetProposals = () => { setError(null); setPlanProposal(null); setCheckProposal(null); setCheckNone(false); setRedactedCount(0); };

  // 目標一改，正在跑的拆解／檢查與未採用的提議都不再有意義。
  const prevGoalRef = useRef(goal);
  useEffect(() => {
    if (prevGoalRef.current === goal) return;
    prevGoalRef.current = goal;
    cancelInFlight();
    resetProposals();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [goal]);
  useEffect(() => () => {
    if (busyRef.current) abortAi(connId(busyRef.current)).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  const commit = (next: Milestone[], forGoal?: string) => {
    if (next.length === 0) onChange(undefined);
    else onChange({ forGoal: forGoal ?? state?.forGoal ?? goal, items: next });
  };

  // ── 手動編輯 ────────────────────────────────────────────
  const startEdit = (id: string, text: string) => { setEditingId(id); setDraft(text); };
  const stopEdit = () => { setEditingId(null); setDraft(""); };
  const saveEdit = (e?: FormEvent) => {
    e?.preventDefault();
    const text = draft.trim();
    if (!text) return;
    if (editingId === "new") commit([...items, newMilestone(text)]);
    else commit(items.map((i) => (i.id === editingId ? { ...i, text: text.slice(0, MAX_MILESTONE_CHARS) } : i)));
    stopEdit();
  };
  const onEditKey = (e: KeyboardEvent<HTMLInputElement>) => { if (e.key === "Escape") stopEdit(); };
  const toggle = (id: string) => commit(items.map((i) => (i.id === id ? { ...i, done: !i.done } : i)));
  const remove = (id: string) => commit(items.filter((i) => i.id !== id));
  const move = (index: number, delta: -1 | 1) => {
    const j = index + delta;
    if (j < 0 || j >= items.length) return;
    const next = [...items];
    [next[index], next[j]] = [next[j], next[index]];
    commit(next);
  };

  // ── AI ─────────────────────────────────────────────────
  const runAi = async (kind: "plan" | "check", content: string, onReply: (text: string | null) => void) => {
    const myReq = ++reqRef.current;
    busyRef.current = kind;
    setBusy(kind);
    try {
      const reply = await invokeAiComplete([{ role: "user", content }], PLAIN_COMPLETION_SYSTEM_PROMPT, connId(kind), providerId);
      if (myReq !== reqRef.current) return;
      onReply(reply.content);
    } catch (e) {
      if (myReq !== reqRef.current) return;
      setError(t.ms_error(describeError(e)));
    } finally {
      if (myReq === reqRef.current) { busyRef.current = null; setBusy(null); }
    }
  };

  const startPlan = async () => {
    if (busyRef.current || !goal.trim()) return;
    resetProposals();
    await runAi("plan", buildMilestonePlanRequest(goal, languageDirective(locale)), (text) => {
      const planned = parseMilestonePlan(text);
      if (planned.length === 0) setError(t.ms_plan_empty);
      else setPlanProposal(planned);
    });
  };

  const startCheck = async () => {
    if (busyRef.current || items.every((i) => i.done)) return;
    resetProposals();
    const screen = normalizeScreen(serializeTerminal(sessionId) ?? "");
    if (!screen) { setError(t.suggest_empty_screen); return; }
    // 先遮罩、再截斷與套預算：截斷若切在 token 中間，會留下一段沒被遮的殘片。
    let maskedCount = 0;
    const mask = (text: string) => {
      if (!redact) return text;
      const r = redactSecrets(text);
      maskedCount += r.count;
      return r.text;
    };
    const aiScreen = mask(screen);
    const history = formatHistoryForPrompt((getHistory?.() ?? []).map(mask), aiScreen, PROMPT_HISTORY_BUDGET);
    const current = aiScreen.length > MAX_SCREEN_CHARS ? aiScreen.slice(-MAX_SCREEN_CHARS) : aiScreen;
    setRedactedCount(maskedCount);
    await runAi(
      "check",
      buildMilestoneCheckRequest(goal, items, history, current, languageDirective(locale)),
      (text) => {
        const r = parseMilestoneCheck(text, items);
        if (r.doneIds.length === 0) setCheckNone(true);
        else setCheckProposal({ ids: r.doneIds, note: r.note });
      },
    );
  };

  const adoptPlan = () => {
    if (!planProposal) return;
    commit(mergePlanKeepingDone(items, planProposal), goal);
    setPlanProposal(null);
  };
  const adoptCheckItem = (id: string) => {
    commit(items.map((i) => (i.id === id ? { ...i, done: true } : i)));
    dropCheckItem(id);
  };
  const dropCheckItem = (id: string) =>
    setCheckProposal((p) => {
      if (!p) return p;
      const ids = p.ids.filter((x) => x !== id);
      return ids.length > 0 ? { ...p, ids } : null;
    });

  const goalChanged = !!state && items.length > 0 && state.forGoal !== goal;
  const editor = (
    <form className="aiterm-ms__edit" onSubmit={saveEdit}>
      <input
        aria-label={t.ms_text_label}
        value={draft}
        placeholder={t.ms_add_placeholder}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={onEditKey}
        autoFocus
      />
      <button type="submit">{t.ms_save}</button>
      <button type="button" onClick={stopEdit}>{t.ms_cancel}</button>
    </form>
  );

  return (
    <section className="aiterm-ms" aria-label={t.ms_title}>
      <header className="aiterm-ms__head">
        <button
          type="button"
          className="aiterm-ms__title"
          aria-expanded={!collapsed}
          onClick={() => setCollapsed((c) => !c)}
        >
          <span>{t.ms_title}</span>
          {items.length > 0 && <span className="aiterm-ms__progress">{t.ms_progress(done, items.length)}</span>}
          <span className={`aiterm-ms__chevron${collapsed ? "" : " aiterm-ms__chevron--open"}`} aria-hidden="true">▸</span>
        </button>
        <div className="aiterm-ms__tools">
          <button
            type="button"
            disabled={!goal.trim() || busy !== null}
            title={goal.trim() ? t.ms_plan_title : t.ms_plan_need_goal}
            onClick={() => void startPlan()}
          >
            {busy === "plan" ? t.ms_planning : t.ms_plan}
          </button>
          <button
            type="button"
            disabled={items.length === 0 || items.every((i) => i.done) || busy !== null}
            title={t.ms_check_title}
            onClick={() => void startCheck()}
          >
            {busy === "check" ? t.ms_checking : t.ms_check}
          </button>
        </div>
      </header>

      {!collapsed && (
        <div className="aiterm-ms__body">
          {goalChanged && (
            <div className="aiterm-ms__notice">
              <span>{t.ms_goal_changed}</span>
              <button type="button" disabled={busy !== null} onClick={() => void startPlan()}>{t.ms_replan}</button>
            </div>
          )}

          {error && <div className="aiterm-ms__error" role="alert">{error}</div>}

          {planProposal && (
            <section className="aiterm-ms__proposal" aria-label={t.ms_plan_proposal}>
              <ol>{planProposal.map((p) => <li key={p}>{p}</li>)}</ol>
              {items.some((i) => i.done) && <p className="aiterm-ms__keep">{t.ms_plan_keep_done}</p>}
              <div className="aiterm-ms__proposal-actions">
                <button type="button" className="aiterm-ms__adopt" onClick={adoptPlan}>{t.ms_adopt}</button>
                <button type="button" onClick={() => setPlanProposal(null)}>{t.ms_cancel}</button>
              </div>
            </section>
          )}

          {checkProposal && (
            <section className="aiterm-ms__proposal" aria-label={t.ms_check_proposal}>
              {checkProposal.note && <p className="aiterm-ms__note">{checkProposal.note}</p>}
              <ul>
                {checkProposal.ids.map((id) => {
                  const text = items.find((i) => i.id === id)?.text ?? "";
                  return (
                    <li key={id}>
                      <span>{text}</span>
                      <button type="button" className="aiterm-ms__adopt" aria-label={t.ms_adopt_item(text)} onClick={() => adoptCheckItem(id)}>{t.ms_adopt}</button>
                      <button type="button" aria-label={t.ms_skip_item(text)} onClick={() => dropCheckItem(id)}>×</button>
                    </li>
                  );
                })}
              </ul>
            </section>
          )}
          {checkNone && <div className="aiterm-ms__hint">{t.ms_check_none}</div>}
          {redactedCount > 0 && <div className="aiterm-ms__redacted">{t.suggest_redacted(redactedCount)}</div>}

          {items.length === 0 && editingId !== "new" && <p className="aiterm-ms__hint">{t.ms_empty}</p>}

          <ul className="aiterm-ms__list">
            {items.map((it, idx) => (
              <li key={it.id} className={`aiterm-ms__item${it.done ? " aiterm-ms__item--done" : ""}${focusId === it.id ? " aiterm-ms__item--focus" : ""}`}>
                {editingId === it.id ? editor : (
                  <>
                    <input
                      type="checkbox"
                      checked={it.done}
                      aria-label={t.ms_item_done(it.text)}
                      onChange={() => toggle(it.id)}
                    />
                    <span className="aiterm-ms__text">{it.text}</span>
                    {focusId === it.id && <span className="aiterm-ms__focus">{t.ms_focus_badge}</span>}
                    {!it.done && focusId !== it.id && (
                      <button
                        type="button"
                        className="aiterm-ms__focus-btn"
                        aria-label={t.ms_focus_set(it.text)}
                        title={t.ms_focus_set(it.text)}
                        onClick={() => onFocus(it.id)}
                      >◎</button>
                    )}
                    <span className="aiterm-ms__row-actions">
                      <button type="button" aria-label={t.ms_item_edit(it.text)} title={t.ms_item_edit(it.text)} onClick={() => startEdit(it.id, it.text)}>✎</button>
                      <button type="button" aria-label={t.ms_item_up(it.text)} disabled={idx === 0} onClick={() => move(idx, -1)}>▲</button>
                      <button type="button" aria-label={t.ms_item_down(it.text)} disabled={idx === items.length - 1} onClick={() => move(idx, 1)}>▼</button>
                      <button type="button" aria-label={t.ms_item_delete(it.text)} onClick={() => remove(it.id)}>×</button>
                    </span>
                  </>
                )}
              </li>
            ))}
          </ul>

          {editingId === "new" ? editor : (
            <button
              type="button"
              className="aiterm-ms__add"
              disabled={items.length >= MAX_MILESTONES}
              onClick={() => startEdit("new", "")}
            >
              {t.ms_add}
            </button>
          )}
        </div>
      )}
    </section>
  );
}
