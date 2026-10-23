import { useEffect, useRef, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { MarkdownText } from "../../lib/markdown";
import { MermaidBlock } from "../MermaidBlock";
import type { QaAnswer } from "./qa";

export type QaMessage =
  | { id: string; role: "user"; text: string }
  | { id: string; role: "ai"; qa: QaAnswer }
  | { id: string; role: "error"; text: string };

export type QaStage = "select" | "answer";

interface Props {
  messages: QaMessage[];
  busy: boolean;
  stage: QaStage;
  elapsed: number;
  onSend: (question: string) => void;
  onCancel: () => void;
  onPlayTour: (messageId: string) => void;
}

/** 畫布下方的問答抽屜；純展示，所有 AI 流程由父層驅動。 */
export function QaPanel({ messages, busy, stage, elapsed, onSend, onCancel, onPlayTour }: Props) {
  const { t } = useLocale();
  const [text, setText] = useState("");
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = listRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages.length, busy]);

  const send = () => {
    const q = text.trim();
    if (!q || busy) return;
    onSend(q);
    setText("");
  };
  const mmss = `${Math.floor(elapsed / 60)}:${String(elapsed % 60).padStart(2, "0")}`;
  const examples = [t.arch_qa_ex1, t.arch_qa_ex2, t.arch_qa_ex3];

  return (
    <section className="arch-qa" aria-label={t.arch_qa_title}>
      <div className="arch-qa-list" ref={listRef}>
        {messages.length === 0 && (
          <div className="arch-qa-hint">
            <p>{t.arch_qa_hint}</p>
            {examples.map((ex) => (
              <button key={ex} type="button" className="arch-btn arch-btn-small arch-qa-example" onClick={() => setText(ex)}>{ex}</button>
            ))}
          </div>
        )}
        {messages.map((m) => {
          if (m.role === "user") return <div key={m.id} className="arch-qa-msg arch-qa-user">{m.text}</div>;
          if (m.role === "error") return <div key={m.id} className="arch-qa-msg arch-qa-err" role="alert">{m.text}</div>;
          const { qa } = m;
          return (
            <div key={m.id} className="arch-qa-msg arch-qa-ai">
              <MarkdownText text={qa.answer} />
              {qa.tour && (
                <button type="button" className="arch-btn arch-btn-small arch-qa-play" onClick={() => onPlayTour(m.id)}>
                  {t.arch_qa_play.replace("{n}", String(qa.tour.steps.length))}
                </button>
              )}
              {qa.diagram && (
                <details className="arch-qa-diagram">
                  <summary>{t.arch_qa_diagram}</summary>
                  <MermaidBlock chart={qa.diagram} />
                </details>
              )}
              {qa.dropped > 0 && <div className="arch-qa-muted">{t.arch_qa_dropped.replace("{n}", String(qa.dropped))}</div>}
            </div>
          );
        })}
      </div>
      {busy && (
        <div className="arch-analyzing arch-qa-busy" role="status">
          <i className="arch-spinner" />
          <span>{stage === "select" ? t.arch_qa_stage_select : t.arch_qa_stage_answer}</span>
          <span>{mmss}</span>
          <button type="button" className="arch-btn arch-btn-small" onClick={onCancel}>{t.arch_cancel}</button>
        </div>
      )}
      <div className="arch-qa-input">
        <textarea
          value={text}
          rows={2}
          placeholder={t.arch_qa_placeholder}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            // Esc 不該連帶關掉檔案預覽或結束導覽。
            if (e.key === "Escape") { e.stopPropagation(); e.nativeEvent.stopImmediatePropagation(); return; }
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) { e.preventDefault(); send(); }
          }}
        />
        <button type="button" className="arch-btn arch-btn-primary" disabled={busy || !text.trim()} onClick={send}>{t.arch_qa_send}</button>
      </div>
    </section>
  );
}
