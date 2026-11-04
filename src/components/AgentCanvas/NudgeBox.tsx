import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

import { useLocale } from "../../contexts/LocaleContext";
import { createImeEnterGuard } from "../../lib/imeComposing";

/** 縮放比例低於這個值就改用精簡模式：節點整個被 world 的 scale() 縮小，行內輸入框與按鈕會小到點不到。 */
export const NUDGE_COMPACT_ZOOM = 0.75;
const POP_WIDTH = 300;
const POP_MARGIN = 8;

/**
 * 執行中的卡片「補充一句」：一行輸入，送出直接進那張卡的 Claude session，不用開分頁、不用停掉重建。
 * 失敗（卡片剛好結束、分頁不在了）由外層顯示原因，這裡保留輸入方便再試。
 *
 * 縮放 < 75% 時只放一顆小按鈕，輸入框改開在 body 底下的浮出框（position: fixed）——
 * 掛在節點裡的東西都會被 scale() 一起縮，浮出框才能維持正常大小。
 */
export function NudgeBox({
  agentId,
  onSend,
  zoom = 1,
}: {
  agentId: string;
  onSend: (message: string) => Promise<void>;
  zoom?: number;
}) {
  const { t } = useLocale();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ left: number; top?: number; bottom?: number } | null>(null);
  const ime = useRef(createImeEnterGuard());
  const btnRef = useRef<HTMLButtonElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  const compact = zoom < NUDGE_COMPACT_ZOOM;
  const popOpen = compact && open;

  const send = async () => {
    const message = text.trim();
    if (!message || busy) return;
    setBusy(true);
    try {
      await onSend(message);
      setText("");
      setOpen(false);
    } catch {
      // 原因由 onSend 的呼叫端顯示；保留輸入，浮出框也維持開啟。
    } finally {
      setBusy(false);
    }
  };

  // 定位在按鈕旁；靠近視窗底部就改開在按鈕上方，左右不超出視窗。
  useLayoutEffect(() => {
    if (!popOpen || !btnRef.current) return;
    const r = btnRef.current.getBoundingClientRect();
    const left = Math.max(POP_MARGIN, Math.min(r.left, window.innerWidth - POP_WIDTH - POP_MARGIN));
    setPos(r.bottom > window.innerHeight - 90 ? { left, bottom: window.innerHeight - r.top + 6 } : { left, top: r.bottom + 6 });
  }, [popOpen]);

  // Esc、點外面、捲動／縮放視窗（按鈕位置會變）都關閉。
  useEffect(() => {
    if (!popOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    const onDown = (e: MouseEvent) => {
      const el = e.target as Node;
      if (popRef.current?.contains(el) || btnRef.current?.contains(el)) return;
      setOpen(false);
    };
    const close = () => setOpen(false);
    document.addEventListener("keydown", onKey);
    document.addEventListener("mousedown", onDown);
    window.addEventListener("resize", close);
    window.addEventListener("scroll", close, true);
    return () => {
      document.removeEventListener("keydown", onKey);
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("resize", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [popOpen]);

  const field = (
    <>
      <input
        className="ac-nudge-input"
        data-testid={`nudge-input-${agentId}`}
        value={text}
        disabled={busy}
        autoFocus={compact}
        placeholder={t.ac_nudge_placeholder}
        title={t.ac_nudge_tip}
        onChange={(e) => setText(e.target.value)}
        onCompositionStart={() => ime.current.onCompositionStart()}
        onCompositionEnd={() => ime.current.onCompositionEnd()}
        onKeyDown={(e) => {
          if (e.key !== "Enter" || ime.current.shouldIgnoreEnter(e)) return;
          e.preventDefault();
          void send();
        }}
      />
      <button
        type="button"
        className="ac-ctlbtn"
        data-testid={`nudge-send-${agentId}`}
        disabled={busy || !text.trim()}
        title={t.ac_nudge_tip}
        onClick={() => void send()}
      >
        {t.ac_nudge_send}
      </button>
    </>
  );

  if (!compact) {
    return (
      <div className="ac-nudge" onMouseDown={(e) => e.stopPropagation()}>
        {field}
      </div>
    );
  }

  return (
    <div className="ac-nudge ac-nudge--compact" onMouseDown={(e) => e.stopPropagation()}>
      <button
        ref={btnRef}
        type="button"
        className="ac-ctlbtn"
        data-testid={`nudge-open-${agentId}`}
        aria-expanded={popOpen}
        title={t.ac_nudge_tip}
        onClick={() => setOpen((o) => !o)}
      >
        {t.ac_nudge_open}
      </button>
      {popOpen &&
        createPortal(
          // React 事件會沿著元件樹穿過 portal 冒泡到桌布：擋住 mousedown，免得在輸入框裡點一下就開始平移桌布。
          <div
            ref={popRef}
            className="ac-nudge-pop"
            data-testid={`nudge-pop-${agentId}`}
            role="dialog"
            aria-label={t.ac_nudge_open}
            style={{ position: "fixed", width: POP_WIDTH, ...(pos ?? { left: POP_MARGIN, top: POP_MARGIN, visibility: "hidden" as const }) }}
            onMouseDown={(e) => e.stopPropagation()}
          >
            {field}
          </div>,
          document.body,
        )}
    </div>
  );
}
