import { useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import type { Reference } from "../../ipc/agents";
import { buildDiscussionMessage, buildDiscussionSystem, parseDecision, type DiscussionTurn, type MethodDecision } from "./methodDecision";

/**
 * 空桌布的「先討論做法」：多輪對話，AI 有把握時提出做法決定，使用者確認後才建團隊。
 * 只對話、不建任何東西；外部呼叫都由 props 注入，測試不必 mock Tauri。
 */

const strip = (reply: string) => reply.replace(/```decision[\s\S]*?```/g, "").trim();

export function MethodDiscussion({
  complete,
  getContext,
  busy,
  confirmLocked = false,
  onConfirm,
  onBack,
}: {
  complete: (messages: { role: "user"; content: string }[], system: string) => Promise<string>;
  getContext: () => Promise<{ brief: string; refs: Reference[] }>;
  /** 外層正在建團隊：鎖住輸入與確認。 */
  busy: boolean;
  /** 只鎖「確認」：討論本身不需要前置條件（例如程式庫還沒選好），建團隊才需要。 */
  confirmLocked?: boolean;
  onConfirm: (d: MethodDecision) => void;
  onBack: () => void;
}) {
  const { t } = useLocale();
  const [turns, setTurns] = useState<DiscussionTurn[]>([]);
  const [text, setText] = useState("");
  const [thinking, setThinking] = useState(false);
  const [decision, setDecision] = useState<MethodDecision | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** 送出當下的歷史；用 ref 避免連按時讀到舊閉包。 */
  const turnsRef = useRef<DiscussionTurn[]>([]);

  const approachLabel = {
    single: t.ac_md_approach_single,
    batch: t.ac_md_approach_batch,
    generator: t.ac_md_approach_generator,
  };

  const send = async () => {
    const content = text.trim();
    if (!content || thinking || busy) return;
    setError(null);
    const next = [...turnsRef.current, { role: "user" as const, content }];
    turnsRef.current = next;
    setTurns(next);
    setText("");
    setThinking(true);
    try {
      const ctx = await getContext();
      const reply = await complete([{ role: "user", content: buildDiscussionMessage(next, ctx.brief, ctx.refs) }], buildDiscussionSystem());
      const withReply = [...next, { role: "assistant" as const, content: strip(reply) }];
      turnsRef.current = withReply;
      setTurns(withReply);
      // 以最新的 decision 區塊為準；這一輪沒附就保留上一個。
      setDecision((prev) => parseDecision(reply) ?? prev);
    } catch (e) {
      setError(t.ac_md_error(e instanceof Error ? e.message : String(e)));
    } finally {
      setThinking(false);
    }
  };

  return (
    <div className="ac-md" data-testid="ac-md">
      <h3 className="ac-req-title">{t.ac_md_title}</h3>
      <div className="ac-md-thread">
        {turns.map((x, i) => (
          <p key={i} className={`ac-md-turn ac-md-turn--${x.role}`}>
            {x.content}
          </p>
        ))}
        {thinking && <p className="ac-md-turn ac-md-turn--assistant">{t.ac_md_thinking}</p>}
      </div>
      {decision && (
        <div className="ac-md-decision" data-testid="ac-md-decision">
          <strong>{t.ac_md_decision_title}：{approachLabel[decision.approach]}</strong>
          {decision.reason && <p>{t.ac_md_reason}：{decision.reason}</p>}
          {decision.trial && <p>{t.ac_md_trial}：{decision.trial}</p>}
          {decision.approach === "batch" && <p className="task-field-hint">{t.ac_md_batch_note}</p>}
        </div>
      )}
      {error && <p className="ac-bad-text">{error}</p>}
      <textarea
        className="ac-req-input"
        data-testid="ac-md-input"
        rows={3}
        value={text}
        placeholder={t.ac_md_placeholder}
        disabled={thinking || busy}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void send();
        }}
      />
      <div className="ac-req-actions">
        <button className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" data-testid="ac-md-back" disabled={busy} onClick={onBack}>
          {t.ac_md_back}
        </button>
        <button className="aiterm-btn aiterm-btn--secondary" data-testid="ac-md-send" disabled={thinking || busy || !text.trim()} onClick={() => void send()}>
          {t.ac_md_send}
        </button>
        <button
          className="aiterm-btn aiterm-btn--primary"
          data-testid="ac-md-confirm"
          title={decision ? undefined : t.ac_md_confirm_hint}
          disabled={!decision || thinking || busy || confirmLocked}
          onClick={() => decision && onConfirm(decision)}
        >
          {t.ac_md_confirm}
        </button>
      </div>
    </div>
  );
}
