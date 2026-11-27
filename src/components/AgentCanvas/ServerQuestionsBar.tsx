import { useCallback, useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { useIntakeRemote } from "../../hooks/useIntakeRemote";
import { answerServerQuestion, cancelServerQuestion, listServerQuestions, questionOptions, type ServerQuestion } from "../../ipc/serverQuestions";

/** 多久問一次 server 有沒有新的待回答問題（毫秒）。 */
const POLL_MS = 3000;

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

/**
 * 共用專案頂列：server 上的 Agent 用 `ask_human` 提問、等人回答時出現「💬 N 個問題待回答」，按下去看內容並回答。
 * 任何成員都能回答（點選項或自己打）；管理員可以取消。沒有待回答的問題時什麼都不畫。
 */
export function ServerQuestionsBar({ projectId, agentName, active }: { projectId: string; agentName: (id: string | null) => string; active: boolean }) {
  const { t } = useLocale();
  const isAdmin = useIntakeRemote().role === "admin";
  const [list, setList] = useState<{ projectId: string; rows: ServerQuestion[] }>({ projectId, rows: [] });
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<number, string>>({});

  const load = useCallback(async () => {
    try {
      const r = await listServerQuestions(projectId);
      setList({ projectId, rows: r.open });
    } catch {
      // 連不上或 server 較舊：保留上一次的清單，等下一輪。
    }
  }, [projectId]);

  useEffect(() => {
    if (!active) return;
    // 第一次查詢排到下一個 tick（effect 裡不同步呼叫會設定 state 的函式）。
    const first = setTimeout(() => void load(), 0);
    const timer = setInterval(() => void load(), POLL_MS);
    return () => {
      clearTimeout(first);
      clearInterval(timer);
    };
  }, [active, load]);

  const rows = list.projectId === projectId ? list.rows : [];
  if (rows.length === 0) return null;

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errText(e));
    } finally {
      await load();
      setBusy(false);
    }
  };
  const answer = (q: ServerQuestion, text: string) => {
    const a = text.trim();
    if (!a) return;
    void run(async () => {
      await answerServerQuestion(q.id, a);
      setDrafts((d) => ({ ...d, [q.id]: "" }));
    });
  };

  return (
    <>
      <button type="button" className="aiterm-btn aiterm-btn--primary aiterm-btn--sm ac-db-approvals-btn" data-testid="ac-questions" onClick={() => setOpen(true)}>
        💬 {t.ac_ask_pending(rows.length)}
      </button>
      {open && (
        <div className="task-dialog-backdrop">
          <div className="task-dialog ac-db-dialog" role="dialog" aria-label={t.ac_ask_title} data-testid="ac-questions-dialog">
            <h3 className="task-dialog-title">{t.ac_ask_title}</h3>
            <p className="ac-dialog-desc">{t.ac_ask_desc}</p>
            <div className="ac-db-conns">
              {rows.map((q) => {
                const opts = questionOptions(q);
                return (
                  <div key={q.id} className="ac-db-approval" data-testid="ac-question">
                    <div className="ac-db-meta">
                      Q-{q.id} · {agentName(q.agent_id)}
                    </div>
                    <div style={{ whiteSpace: "pre-wrap" }}>{q.question}</div>
                    {q.context && <div className="ac-dialog-desc">{t.ac_ask_context(q.context)}</div>}
                    {q.recommended && <div className="ac-dialog-desc">{t.ac_ask_recommended(q.recommended)}</div>}
                    {opts.length > 0 && (
                      <div className="ac-db-approval-actions">
                        {opts.map((o) => (
                          <button key={o} className={`aiterm-btn aiterm-btn--sm ${o === q.recommended ? "aiterm-btn--primary" : "aiterm-btn--secondary"}`} disabled={busy} onClick={() => answer(q, o)}>
                            {o}
                          </button>
                        ))}
                      </div>
                    )}
                    <div className="ac-db-approval-actions">
                      <input
                        className="aiterm-input"
                        style={{ flex: 1 }}
                        aria-label={t.ac_ask_answer_ph}
                        placeholder={t.ac_ask_answer_ph}
                        value={drafts[q.id] ?? ""}
                        onChange={(e) => setDrafts((d) => ({ ...d, [q.id]: e.target.value }))}
                        onKeyDown={(e) => e.key === "Enter" && !e.nativeEvent.isComposing && answer(q, drafts[q.id] ?? "")}
                      />
                      <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" disabled={busy || !(drafts[q.id] ?? "").trim()} onClick={() => answer(q, drafts[q.id] ?? "")}>
                        {t.ac_ask_send}
                      </button>
                      {isAdmin && (
                        <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={() => void run(() => cancelServerQuestion(q.id))}>
                          {t.ac_ask_cancel}
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
            {error && (
              <p className="ac-bad-text" role="alert">
                {error}
              </p>
            )}
            <div className="task-dialog-actions">
              <button className="aiterm-btn aiterm-btn--secondary" onClick={() => setOpen(false)}>
                {t.ac_done_close}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
