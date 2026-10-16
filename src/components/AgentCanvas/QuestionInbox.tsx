import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { optionList, type QuestionView } from "../../ipc/questions";

/**
 * 「待決策」收件匣：Agent 用 ask_human 提出、還沒回答的問題。按選項或自己打字回答，
 * 答案會送回提問的 Agent（它接著做）。Telegram 上回答的也會從這裡消失。
 */
export function QuestionInbox({
  questions,
  focusId,
  onAnswer,
  onClose,
}: {
  questions: QuestionView[];
  /** 從 Agent 卡點進來時，標示那一題。 */
  focusId: number | null;
  onAnswer: (q: QuestionView, answer: string) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  return (
    <aside className="ac-inbox" role="dialog" aria-label={t.ac_inbox_title}>
      <div className="ac-inbox-head">
        <h2>{t.ac_inbox_title}</h2>
        <button className="ac-iconbtn" aria-label={t.ac_inbox_close} title={t.ac_inbox_close} onClick={onClose}>
          ×
        </button>
      </div>
      {questions.length === 0 ? (
        <div className="ac-inbox-empty">{t.ac_inbox_empty}</div>
      ) : (
        <div className="ac-inbox-list">
          {questions.map((q) => (
            <QuestionItem key={`${q.project_id}:${q.id}`} q={q} focused={q.id === focusId} onAnswer={onAnswer} />
          ))}
        </div>
      )}
    </aside>
  );
}

function QuestionItem({
  q,
  focused,
  onAnswer,
}: {
  q: QuestionView;
  focused: boolean;
  onAnswer: (q: QuestionView, answer: string) => Promise<void>;
}) {
  const { t } = useLocale();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const send = async (answer: string) => {
    if (!answer.trim() || busy) return;
    setBusy(true);
    try {
      await onAnswer(q, answer.trim());
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className={`ac-q${focused ? " ac-q--focus" : ""}`} data-testid={`question-${q.id}`}>
      <div className="ac-q-meta">
        Q-{q.id} · {q.project_name} · {q.agent_name ?? "?"} · {q.task_title}
      </div>
      <div className="ac-q-text">{q.question}</div>
      {q.context && <div className="ac-q-context">{q.context}</div>}
      <div className="ac-q-options">
        {optionList(q).map((o) => (
          <button
            key={o}
            className={`aiterm-btn aiterm-btn--sm ${o === q.recommended ? "aiterm-btn--primary" : "aiterm-btn--secondary"}`}
            disabled={busy}
            title={o === q.recommended ? t.ac_q_recommended : undefined}
            onClick={() => void send(o)}
          >
            {o === q.recommended ? "★ " : ""}
            {o}
          </button>
        ))}
      </div>
      <div className="ac-q-free">
        <textarea
          className="task-field-input"
          rows={2}
          value={text}
          placeholder={t.ac_q_free_placeholder}
          aria-label={t.ac_q_free_placeholder}
          onChange={(e) => setText(e.target.value)}
        />
        <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy || !text.trim()} onClick={() => void send(text)}>
          {t.ac_q_send}
        </button>
      </div>
    </div>
  );
}
