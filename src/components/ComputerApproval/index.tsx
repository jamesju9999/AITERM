import { useEffect, useRef, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import {
  onComputerApproval,
  resolveComputerApproval,
  type ApprovalDecision,
  type ComputerApprovalRequest,
} from "../../ipc/computer";
import { unlistenOnCleanup } from "../../lib/eventSubscription";
import "../CloseConfirmDialog/styles.css";

/**
 * AI 要操作其他 App 時的授權對話框。掛在永遠存在的外殼（TerminalApp）：
 * 請求多半來自外部 MCP 客戶端，不能等某個分頁剛好掛載才聽得到。
 * 同時來多個請求就排隊，一次顯示一個。
 */
export function ComputerApprovalDialog() {
  const { t } = useLocale();
  const [queue, setQueue] = useState<ComputerApprovalRequest[]>([]);
  // 同一個事件被處理兩次的保險（見 TerminalApp 對 unlisten 失敗的註解）。
  const seenRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    return unlistenOnCleanup(
      onComputerApproval((req) => {
        if (seenRef.current.has(req.id)) return;
        seenRef.current.add(req.id);
        setQueue((q) => [...q, req]);
      }),
      "computer://approval-request",
    );
  }, []);

  const current = queue[0];
  if (!current) return null;

  const answer = (decision: ApprovalDecision) => {
    // 就算回覆失敗（已逾時），也要關掉對話框，否則會卡住後面的請求。
    void resolveComputerApproval(current.id, decision).catch(() => {});
    setQueue((q) => q.slice(1));
  };

  const destructive = current.kind === "destructive";
  const title = (destructive ? t.computer_approval_destructive_title : t.computer_approval_first_title)
    .replace("{app}", current.app);
  const body = (destructive ? t.computer_approval_destructive_body : t.computer_approval_first_body)
    .replace("{app}", current.app)
    .replace("{action}", current.action)
    .replace("{detail}", current.detail);

  return (
    <div className="aiterm-close-overlay" role="dialog" aria-modal="true" aria-label={title}>
      <div className="aiterm-close-dialog">
        <h3 className="aiterm-close-dialog-title">{title}</h3>
        <div className="aiterm-close-dialog-body">{body}</div>
        <div className="aiterm-close-dialog-actions">
          <button type="button" className="aiterm-btn aiterm-btn--secondary" onClick={() => answer("deny")}>
            {t.computer_approval_deny}
          </button>
          <button type="button" className="aiterm-btn aiterm-btn--secondary" onClick={() => answer("allow_once")}>
            {destructive ? t.computer_approval_allow : t.computer_approval_once}
          </button>
          {!destructive && (
            <button type="button" className="aiterm-btn aiterm-btn--danger-solid" onClick={() => answer("allow_always")}>
              {t.computer_approval_always}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
