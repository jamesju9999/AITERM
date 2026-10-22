import { useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { SparklesIcon } from "../Icons";
import type { StartResult } from "./startFromRequirement";

/**
 * 需求先行組好團隊後的說明：預設只露出一行摘要，各角色的理由收在「查看理由」裡，
 * 不然長長一大塊會蓋住剛放上桌布的團隊。
 */
export function StartNotice({ result, onClose }: { result: StartResult; onClose: () => void }) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
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
