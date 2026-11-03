import { useEffect, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { parseDecisionJson, type MethodDecision } from "./methodDecision";

/**
 * 空桌布的「先討論做法」：開一個 Claude Code 分頁讓使用者與 AI 討論，
 * AI 把決定寫進 JSON 檔，這裡輪詢讀取並顯示決定卡；使用者確認後才建團隊。
 * 外部呼叫都由 props 注入，測試不必 mock Tauri。
 */
export function MethodDiscussion({
  start,
  read,
  busy,
  confirmLocked = false,
  onConfirm,
  onBack,
  openTab,
  onDecisionReady,
  pollMs = 2000,
}: {
  start: (opening: string) => Promise<{ discussionId: string; tabId: string }>;
  read: (discussionId: string) => Promise<string | null>;
  /** 外層正在建團隊：鎖住輸入與確認。 */
  busy: boolean;
  /** 只鎖「確認」：討論本身不需要前置條件（例如程式庫還沒選好），建團隊才需要。 */
  confirmLocked?: boolean;
  onConfirm: (d: MethodDecision) => void;
  onBack: () => void;
  /** 切到討論用的 Claude Code 分頁（開始後自動呼叫一次，也給「前往討論分頁」鈕用）。 */
  openTab?: (tabId: string) => void;
  /** 這一輪討論第一次讀到決定時呼叫一次（使用者多半正在 Claude Code 分頁裡，要把他叫回來）。 */
  onDecisionReady?: () => void;
  pollMs?: number;
}) {
  const { t } = useLocale();
  const [opening, setOpening] = useState("");
  const [starting, setStarting] = useState(false);
  const [discussionId, setDiscussionId] = useState<string | null>(null);
  const [tabId, setTabId] = useState<string | null>(null);
  const [decision, setDecision] = useState<MethodDecision | null>(null);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);
  /** 外層常傳行內函式；用 ref 讀最新的，輪詢才不會每次重繪就重設計時器。 */
  const readRef = useRef(read);
  const openTabRef = useRef(openTab);
  const onDecisionReadyRef = useRef(onDecisionReady);
  /** 這一輪討論是否已經通知過「決定出爐」。 */
  const announcedRef = useRef(false);
  useEffect(() => {
    readRef.current = read;
    openTabRef.current = openTab;
    onDecisionReadyRef.current = onDecisionReady;
  });

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  // 有討論 id 就輪詢；決定卡出現後仍繼續（AI 可能覆寫）。重新開始或卸載時清掉。
  useEffect(() => {
    if (!discussionId) return;
    let alive = true;
    const tick = async () => {
      try {
        const text = await readRef.current(discussionId);
        if (!alive || !mountedRef.current || text == null) return;
        const d = parseDecisionJson(text);
        if (d) {
          setDecision(d);
          if (!announcedRef.current) {
            announcedRef.current = true;
            onDecisionReadyRef.current?.();
          }
        }
      } catch {
        // 讀不到就當還沒寫好，下一輪再試
      }
    };
    const timer = setInterval(() => void tick(), pollMs);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [discussionId, pollMs]);

  const begin = async () => {
    const text = opening.trim();
    if (!text || starting || busy) return;
    setError(null);
    setStarting(true);
    try {
      const r = await start(text);
      if (mountedRef.current) {
        announcedRef.current = false;
        setDiscussionId(r.discussionId);
        setTabId(r.tabId);
        openTabRef.current?.(r.tabId);
      }
    } catch (e) {
      if (mountedRef.current) setError(t.ac_md_error(e instanceof Error ? e.message : String(e)));
    } finally {
      if (mountedRef.current) setStarting(false);
    }
  };

  const restart = () => {
    setDiscussionId(null);
    setTabId(null);
    setDecision(null);
  };

  const approachLabel = {
    single: t.ac_md_approach_single,
    batch: t.ac_md_approach_batch,
    generator: t.ac_md_approach_generator,
  };

  return (
    <div className="ac-md" data-testid="ac-md">
      {discussionId === null ? (
        <>
          <p className="ac-md-intro">{t.ac_md_intro}</p>
          <textarea
            className="ac-req-input"
            data-testid="ac-md-input"
            rows={4}
            value={opening}
            placeholder={t.ac_md_placeholder}
            disabled={starting || busy}
            onChange={(e) => setOpening(e.target.value)}
          />
          {error && <p className="ac-bad-text">{error}</p>}
          <div className="ac-req-actions">
            <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" data-testid="ac-md-back" disabled={busy} onClick={onBack}>
              {t.ac_md_back}
            </button>
            <button
              className="aiterm-btn aiterm-btn--primary"
              data-testid="ac-md-start"
              disabled={starting || busy || !opening.trim()}
              onClick={() => void begin()}
            >
              {starting ? t.ac_md_starting : t.ac_md_start}
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="ac-md-status" data-testid="ac-md-status">{t.ac_md_running}</p>
          {tabId && openTab && (
            <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" data-testid="ac-md-open" onClick={() => openTab(tabId)}>
              {t.ac_md_open_tab}
            </button>
          )}
          {decision && (
            <div className="ac-md-decision" data-testid="ac-md-decision">
              <strong>{t.ac_md_decision_title}：{approachLabel[decision.approach]}</strong>
              {decision.reason && <p>{t.ac_md_reason}：{decision.reason}</p>}
              {decision.trial && <p>{t.ac_md_trial}：{decision.trial}</p>}
              {decision.approach === "batch" && <p className="task-field-hint">{t.ac_md_batch_note}</p>}
            </div>
          )}
          <div className="ac-req-actions">
            <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" data-testid="ac-md-restart" disabled={busy} onClick={restart}>
              {t.ac_md_restart}
            </button>
            <button
              className="aiterm-btn aiterm-btn--primary"
              data-testid="ac-md-confirm"
              title={decision ? undefined : t.ac_md_confirm_hint}
              disabled={!decision || busy || confirmLocked}
              onClick={() => decision && onConfirm(decision)}
            >
              {t.ac_md_confirm}
            </button>
          </div>
        </>
      )}
    </div>
  );
}
