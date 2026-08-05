import { useEffect, useState } from "react";
import {
  claudeNotifEnableBell,
  claudeNotifNeedsPrompt,
  isClaudeNotifDeclined,
  isOnboardingDone,
  setClaudeNotifDeclined,
} from "../ipc/config";
import { useLocale } from "../contexts/LocaleContext";
// Deliberate reuse: this is the same bottom-right card as the update toast and
// the AppImage prompt, and a third copy of those rules would drift from them.
import "./UpdateModal.css";

interface Props {
  /** 這個 session 已經偵測到使用者執行 claude。 */
  claudeSeen: boolean;
  /** 有更優先的角落卡片正在顯示（更新提示或 AppImage 提示）。 */
  blocked: boolean;
}

export function ClaudeNotifPrompt({ claudeSeen, blocked }: Props) {
  const { t } = useLocale();
  const [offer, setOffer] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    // 沒偵測到 claude 就完全不查——沒有理由去碰使用者的設定檔。
    if (!claudeSeen) return;
    let cancelled = false;
    (async () => {
      try {
        const [needs, declined, onboarded] = await Promise.all([
          claudeNotifNeedsPrompt(),
          isClaudeNotifDeclined(),
          isOnboardingDone(),
        ]);
        if (cancelled) return;
        setOffer(needs && !declined && onboarded);
      } catch {
        // Best-effort: a failure here must never block the app.
      }
    })();
    return () => { cancelled = true; };
    // claudeSeen 只會 false → true 一次，所以這個查詢一個 session 只跑一次。
  }, [claudeSeen]);

  if (!offer || blocked) return null;

  const accept = async () => {
    try {
      await claudeNotifEnableBell();
      setDone(true);
    } catch (e) {
      setError(String(e));
    }
  };

  const decline = async () => {
    setOffer(false);
    await setClaudeNotifDeclined().catch(() => {});
  };

  return (
    <div className="update-modal-backdrop">
      <div className="update-modal" role="status" aria-label={t.claude_notif_title}>
        <p className="update-modal-title">{t.claude_notif_title}</p>
        {done ? (
          <>
            {/* 這句不能省：Claude Code 只在啟動時讀設定，不講的話使用者會
                以為設了沒用——我們自己驗證時就踩過這一步。 */}
            <p className="update-modal-notes">{t.claude_notif_done}</p>
            <div className="update-modal-actions">
              <button className="aiterm-btn aiterm-btn--primary" onClick={() => setOffer(false)}>
                {t.claude_notif_dismiss}
              </button>
            </div>
          </>
        ) : (
          <>
            <p className="update-modal-notes">{t.claude_notif_body}</p>
            <p className="update-modal-notes">{t.claude_notif_detail}</p>
            {error && <p className="update-modal-error">{error}</p>}
            <div className="update-modal-actions">
              <button className="aiterm-btn aiterm-btn--secondary" onClick={() => void decline()}>
                {t.claude_notif_decline}
              </button>
              <button className="aiterm-btn aiterm-btn--primary" onClick={() => void accept()}>
                {t.claude_notif_enable}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
