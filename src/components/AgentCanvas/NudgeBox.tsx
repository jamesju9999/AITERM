import { useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { createImeEnterGuard } from "../../lib/imeComposing";

/**
 * 執行中的卡片「補充一句」：一行輸入，送出直接進那張卡的 Claude session，不用開分頁、不用停掉重建。
 * 失敗（卡片剛好結束、分頁不在了）由外層顯示原因，這裡保留輸入方便再試。
 */
export function NudgeBox({ agentId, onSend }: { agentId: string; onSend: (message: string) => Promise<void> }) {
  const { t } = useLocale();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const ime = useRef(createImeEnterGuard());

  const send = async () => {
    const message = text.trim();
    if (!message || busy) return;
    setBusy(true);
    try {
      await onSend(message);
      setText("");
    } catch {
      // 原因由 onSend 的呼叫端顯示；保留輸入。
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="ac-nudge" onMouseDown={(e) => e.stopPropagation()}>
      <input
        className="ac-nudge-input"
        data-testid={`nudge-input-${agentId}`}
        value={text}
        disabled={busy}
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
    </div>
  );
}
