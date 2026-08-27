import { useEffect, useState } from "react";
import { onSharePendingRequest, shareApprove, shareDeny, type PendingRequest } from "../../ipc/share";
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

interface Props {
  /** 分頁清單，用來把 `tabId` 換成使用者看得懂的標題。 */
  tabs: Array<{ id: string; title: string }>;
}

/**
 * 同意視窗：有人要連進來時跳出來。
 *
 * **必須掛在 `TerminalApp` 層，不能在 `TerminalView` 裡。** 非作用中的分頁
 * 是用 `visibility: hidden` + `pointerEvents: none` 隱藏的，而連線請求可能
 * 來自任何一個分享中的分頁——掛在 `TerminalView` 裡的話，對方在非作用分頁
 * 發起連線時，這個視窗看不見也點不到。
 *
 * **這個視窗絕不顯示主控端自己算出的 4 位驗證碼。** 使用者必須跟對方口頭
 * 核對、把聽到的數字打進來。後端根本不送那個值過來（`PendingRequest` 型別
 * 上就沒有），所以這不是「UI 選擇不顯示」，是拿不到。
 *
 * 若碼顯示在這裡，使用者會照抄畫面上的數字而不問對方，人工核對變成自欺，
 * 而那次口頭核對正是整個防中間人保證的最後一哩。
 */
export function ConsentDialog({ tabs }: Props) {
  const { t } = useLocale();
  const [req, setReq] = useState<PendingRequest | null>(null);
  const [typed, setTyped] = useState("");
  const [outcome, setOutcome] = useState<string | null>(null);

  useEffect(() => {
    let un: (() => void) | null = null;
    let disposed = false;
    void onSharePendingRequest((p) => {
      setReq(p);
      setTyped("");
      setOutcome(null);
    }).then((f) => {
      if (disposed) f();
      else un = f;
    });
    return () => {
      disposed = true;
      un?.();
    };
  }, []);

  if (!req) return null;

  const tabTitle = tabs.find((x) => x.id === req.tabId)?.title ?? req.tabId;
  const title = t.consent_title.replace("{name}", req.displayName).replace("{tab}", tabTitle);

  async function decide(mode: "read_only" | "control") {
    const d = await shareApprove(req!.requestId, mode, typed);
    switch (d.kind) {
      case "approved":
        setReq(null);
        return;
      case "codeMismatch":
        // 後端已經拒絕了這筆請求，不給重試。關掉輸入框但留著訊息。
        setOutcome(t.consent_code_mismatch);
        return;
      case "controlTaken":
        // 請求還在，主控端可以改選「只能看」。
        setOutcome(t.consent_control_taken);
        return;
      case "requestGone":
        setOutcome(t.consent_request_gone);
        return;
    }
  }

  const closed = outcome === t.consent_code_mismatch || outcome === t.consent_request_gone;

  return (
    <div className="aiterm-consent__backdrop">
      <div className="aiterm-consent" role="dialog" aria-modal="true">
        <div className="aiterm-consent__title">{title}</div>
        <div className="aiterm-consent__unverified">{t.consent_name_unverified}</div>

        {!closed && (
          <>
            <label className="aiterm-consent__prompt" htmlFor="aiterm-consent-code">
              {t.consent_prompt}
            </label>
            <input
              id="aiterm-consent-code"
              className="aiterm-consent__input"
              type="text"
              inputMode="numeric"
              maxLength={4}
              autoFocus
              value={typed}
              onChange={(e) => setTyped(e.target.value.replace(/\D/g, "").slice(0, 4))}
            />
            <div className="aiterm-consent__warning">⚠️ {t.consent_warning}</div>
          </>
        )}

        {outcome && <div className="aiterm-consent__outcome">{outcome}</div>}

        <div className="aiterm-consent__actions">
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            onClick={() => {
              void shareDeny(req.requestId);
              setReq(null);
            }}
          >
            {closed ? t.connect_cancel : t.consent_deny}
          </button>
          {!closed && (
            <>
              <button
                className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                disabled={typed.length !== 4}
                onClick={() => void decide("read_only")}
              >
                {t.consent_read_only}
              </button>
              <button
                className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                disabled={typed.length !== 4}
                onClick={() => void decide("control")}
              >
                {t.consent_control}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
