import { useEffect, useRef, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { createImeEnterGuard } from "../../lib/imeComposing";
import { MarkdownText } from "../../lib/markdown";
import { MermaidBlock } from "../MermaidBlock";
import type { QaAnswer } from "./qa";

export type QaMessage =
  | { id: string; role: "user"; text: string }
  | { id: string; role: "ai"; qa: QaAnswer }
  | { id: string; role: "error"; text: string; raw?: string };

const RAW_MAX = 4000;

/** 錯誤泡泡裡可展開的 AI 原始回應，方便使用者貼回來除錯。 */
function RawReply({ raw }: { raw: string }) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const cut = raw.length > RAW_MAX;
  return (
    <>
      <button type="button" className="arch-btn arch-btn-small arch-qa-raw-toggle" aria-expanded={open} onClick={() => setOpen((o) => !o)}>
        {open ? t.arch_qa_raw_hide : t.arch_qa_raw_show}
      </button>
      {open && (
        <>
          <pre className="arch-qa-raw" data-testid="qa-raw">{cut ? raw.slice(0, RAW_MAX) : raw}</pre>
          {cut && <div className="arch-qa-raw-note">{t.arch_qa_raw_truncated.replace("{n}", String(raw.length)).replace("{max}", String(RAW_MAX))}</div>}
        </>
      )}
    </>
  );
}

export type QaStage = "select" | "answer";

interface Props {
  messages: QaMessage[];
  busy: boolean;
  stage: QaStage;
  elapsed: number;
  onSend: (question: string) => void;
  onCancel: () => void;
  onPlayTour: (messageId: string) => void;
  onExport: () => void;
  onClear: () => void;
}

/** 畫布下方的問答抽屜；純展示，所有 AI 流程由父層驅動。 */
export function QaPanel({ messages, busy, stage, elapsed, onSend, onCancel, onPlayTour, onExport, onClear }: Props) {
  const { t } = useLocale();
  const [text, setText] = useState("");
  const listRef = useRef<HTMLDivElement>(null);
  const ime = useRef(createImeEnterGuard());
  const [confirmClear, setConfirmClear] = useState(false);
  const [histOpen, setHistOpen] = useState(false);
  const questions = messages.filter((m): m is Extract<QaMessage, { role: "user" }> => m.role === "user");

  useEffect(() => {
    if (!histOpen) return;
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setHistOpen(false); };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [histOpen]);

  const jumpTo = (id: string) => {
    setHistOpen(false);
    listRef.current?.querySelector<HTMLElement>(`[data-mid="${id}"]`)?.scrollIntoView?.({ block: "start", behavior: "smooth" });
  };

  // 清除會刪掉存在專案裡的記錄：第一下只進入確認狀態，3 秒內再按一次才清；不用 window.confirm（Tauri 裡不可靠）。
  useEffect(() => {
    if (!confirmClear) return;
    const id = setTimeout(() => setConfirmClear(false), 3000);
    return () => clearTimeout(id);
  }, [confirmClear]);

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
      <div className="arch-qa-bar">
        <button type="button" className="arch-btn arch-btn-small" aria-expanded={histOpen} disabled={questions.length === 0} onClick={() => setHistOpen((o) => !o)}>
          {t.arch_qa_history.replace("{n}", String(questions.length))}
        </button>
        {messages.length > 0 && (
          <>
            <button type="button" className="arch-btn arch-btn-small" onClick={onExport}>{t.arch_qa_export}</button>
            <button type="button" className={`arch-btn arch-btn-small ${confirmClear ? "arch-btn-danger" : ""}`} disabled={busy}
              onClick={() => { if (confirmClear) { setConfirmClear(false); onClear(); } else setConfirmClear(true); }}>
              {confirmClear ? t.arch_qa_clear_confirm : t.arch_qa_clear}
            </button>
          </>
        )}
      </div>
      {histOpen && questions.length > 0 && (
        <ul className="arch-qa-hist" aria-label={t.arch_qa_history.replace("{n}", String(questions.length))}>
          {[...questions].reverse().map((q) => (
            <li key={q.id}>
              <button type="button" className="arch-qa-hist-q" title={q.text} onClick={() => jumpTo(q.id)}>{q.text}</button>
              <button type="button" className="arch-btn arch-btn-small" onClick={() => { setText(q.text); setHistOpen(false); }}>{t.arch_qa_reask}</button>
            </li>
          ))}
        </ul>
      )}
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
          if (m.role === "user") return <div key={m.id} data-mid={m.id} className="arch-qa-msg arch-qa-user">{m.text}</div>;
          if (m.role === "error") return (
            <div key={m.id} className="arch-qa-msg arch-qa-err" role="alert">
              {m.text}
              {m.raw !== undefined && <RawReply raw={m.raw} />}
            </div>
          );
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
              {qa.truncated && <div className="arch-qa-muted">{t.arch_qa_truncated_warn}</div>}
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
          onCompositionStart={() => ime.current.onCompositionStart()}
          onCompositionEnd={() => ime.current.onCompositionEnd()}
          onKeyDown={(e) => {
            // Esc 不該連帶關掉檔案預覽或結束導覽。
            if (e.key === "Escape") { e.stopPropagation(); e.nativeEvent.stopImmediatePropagation(); return; }
            if (e.key === "Enter" && !e.shiftKey && !ime.current.shouldIgnoreEnter(e)) { e.preventDefault(); send(); }
          }}
        />
        <button type="button" className="arch-btn arch-btn-primary" disabled={busy || !text.trim()} onClick={send}>{t.arch_qa_send}</button>
      </div>
    </section>
  );
}
