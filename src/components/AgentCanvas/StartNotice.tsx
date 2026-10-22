import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { SparklesIcon } from "../Icons";
import type { StartResult } from "./startFromRequirement";

/**
 * 需求先行組好團隊後的說明：預設只露出一行摘要，各角色的理由收在「查看理由」裡，
 * 不然長長一大塊會蓋住剛放上桌布的團隊。
 */
export function StartNotice({
  result,
  onClose,
  onStart,
}: {
  result: StartResult;
  onClose: () => void;
  /** 有還沒派出去的需求卡時，按「開始執行」呼叫；失敗丟例外，訊息會顯示在這裡。 */
  onStart?: (pending: NonNullable<StartResult["pending"]>) => Promise<void>;
}) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const [starting, setStarting] = useState(false);
  const [startError, setStartError] = useState<string | null>(null);
  const hasReasons = !result.usedDefault && result.reasons.length > 0;
  return (
    <div className="ac-req-notice" data-testid="ac-req-notice" role="status">
      <span className="ac-req-notice-icon" aria-hidden>
        <SparklesIcon size={14} />
      </span>
      <div className="ac-req-notice-main">
        <b className="ac-req-notice-title">{result.usedDefault ? t.ac_req_done_default : t.ac_req_done_title}</b>
        {!result.usedDefault && result.summary && <p className="ac-req-notice-sum">{result.summary}</p>}
        {hasReasons && (
          <button type="button" className="ac-req-notice-toggle" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
            {open ? t.ac_req_reasons_hide : t.ac_req_reasons_show} {open ? "▴" : "▾"}
          </button>
        )}
        {result.pending && onStart && (
          <div className="ac-req-notice-pending" data-testid="ac-req-notice-pending">
            <p>{t.ac_req_pending_hint}</p>
            {startError && <p className="ac-bad-text">{startError}</p>}
            <button
              type="button"
              className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
              data-testid="ac-req-start-run"
              disabled={starting}
              onClick={async () => {
                if (!result.pending) return;
                setStarting(true);
                setStartError(null);
                try {
                  await onStart(result.pending);
                } catch (e) {
                  setStartError(e instanceof Error ? e.message : String(e));
                  setStarting(false);
                }
              }}
            >
              {starting ? t.ac_req_starting : t.ac_req_start_run}
            </button>
          </div>
        )}
        {result.criteria.length > 0 && (
          <div className="ac-req-notice-criteria" data-testid="ac-req-notice-criteria">
            <b>{t.ac_req_criteria_title}</b>
            <ul>
              {result.criteria.map((c) => (
                <li key={c}>{c}</li>
              ))}
            </ul>
          </div>
        )}
        {hasReasons && open && (
          <dl className="ac-req-notice-list" data-testid="ac-req-notice-reasons">
            {result.reasons.map((r) => (
              <div key={r.label} className="ac-req-notice-row">
                <dt>{r.label}</dt>
                <dd>{r.reason}</dd>
              </div>
            ))}
          </dl>
        )}
      </div>
      <button className="ac-req-notice-close" aria-label={t.ac_req_dismiss} title={t.ac_req_dismiss} onClick={onClose}>
        ×
      </button>
    </div>
  );
}
