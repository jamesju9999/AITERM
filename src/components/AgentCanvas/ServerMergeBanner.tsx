import { useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { approveMerge } from "../../ipc/tasks";

/**
 * server 編排跑完（所有模組審查通過）後，頂列下面的一條橫幅：說明現在要做什麼，並給「核准合併」。
 * 合併會動到專案資料夾，所以要按兩下（第一下只問確認）；只有管理員能核准，被拒的原因交給 onError。
 */
export function ServerMergeBanner({
  mergeCards,
  onError,
  onNotice,
}: {
  /** 等待核准的合併卡（`card_kind = "merge"` 且還在規劃中）。 */
  mergeCards: Array<{ id: string; title: string }>;
  onError: (e: unknown) => void;
  onNotice: (message: string) => void;
}) {
  const { t } = useLocale();
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState<string | null>(null);
  if (mergeCards.length === 0) return null;

  const approve = async (id: string) => {
    if (confirming !== id) {
      setConfirming(id);
      return;
    }
    setConfirming(null);
    setBusy(true);
    try {
      onNotice(t.ac_server_merge_done(await approveMerge(id)));
    } catch (e) {
      onError(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="ac-merge-banner" data-testid="ac-merge-banner">
      <span className="ac-merge-banner-text">{t.ac_merge_hint}</span>
      {mergeCards.map((c) => (
        <button
          key={c.id}
          className="aiterm-btn aiterm-btn--sm aiterm-btn--primary"
          data-testid="ac-merge-approve"
          disabled={busy}
          onClick={() => void approve(c.id)}
          onBlur={() => setConfirming((cur) => (cur === c.id ? null : cur))}
        >
          {confirming === c.id ? t.ac_merge_confirm : t.ac_merge_approve}
        </button>
      ))}
    </div>
  );
}
