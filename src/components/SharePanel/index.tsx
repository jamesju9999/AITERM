import { useState } from "react";
import { useShareHost } from "../../hooks/useShareHost";
import { useLocale } from "../../contexts/LocaleContext";
import { LinkIcon } from "../Icons";
import "./index.css";

interface Props {
  /** **PTY session id，不是 React 的分頁 id。** 後端拿它去 PtyManager 查串流。 */
  sessionId: string;
}

/**
 * 分享按鈕與展開的面板。
 *
 * 掛在 `TerminalView` 的工具列——那個元件只在終端機分頁渲染，所以
 * spec 的「非終端機分頁隱藏」自動成立，不需要額外的型別判斷。
 *
 * **這個元件不可能顯示主控端的 4 位驗證碼**：那個值根本不會到前端
 * （見 `src/ipc/share.ts` 的 `PendingRequest`）。面板上的 6 位短碼是
 * 另一回事——那是要給對方輸入的，本來就該顯示。
 */
export function SharePanel({ sessionId }: Props) {
  const { t } = useLocale();
  const { sharing, code, port, lanAddress, viewers, start, stop, kick, revokeControl } =
    useShareHost(sessionId);
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);

  // 位址由後端提供（見 Task 3 Step 5）。前端查不到區網 IP——`hostname()`
  // 回的是主機名稱不是位址，而使用者要唸給同事的是 `192.168.1.33:47823`。
  // `lanAddress` 查不到時不是錯誤（見 share.rs 的 `lan_address()`）——退成
  // 只顯示 port，使用者自己知道 IP，也還能唸出來。
  const address = port ? (lanAddress ? `${lanAddress}:${port}` : `${port}`) : null;

  async function onButtonClick() {
    if (!sharing) await start();
    setOpen((v) => !v);
  }

  async function onCopy() {
    if (!address) return;
    await navigator.clipboard.writeText(address);
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }

  return (
    <span className="aiterm-share">
      <button
        className={`aiterm-btn aiterm-btn--secondary aiterm-btn--sm ${sharing ? "aiterm-share__btn--on" : ""}`}
        title={t.share_button_tooltip}
        onClick={(e) => {
          e.stopPropagation();
          void onButtonClick();
        }}
        style={{ display: "flex", alignItems: "center", gap: "6px" }}
      >
        <LinkIcon size={14} />
        <span>{t.share_button}</span>
      </button>

      {open && sharing && (
        <div className="aiterm-share__panel" onClick={(e) => e.stopPropagation()}>
          <div className="aiterm-share__title">{t.share_panel_title}</div>

          <div className="aiterm-share__row">
            <span className="aiterm-share__label">{t.share_panel_code}</span>
            <strong className="aiterm-share__code">{code}</strong>
          </div>

          <div className="aiterm-share__row">
            <span className="aiterm-share__label">{t.share_panel_address}</span>
            <span className="aiterm-share__addr">{address}</span>
            <button
              className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
              onClick={() => void onCopy()}
            >
              {copied ? t.share_panel_copied : t.share_panel_copy}
            </button>
          </div>

          {/* 系統詢問就是這一刻跳出來的——先講，免得使用者被嚇到按拒絕。 */}
          <div className="aiterm-share__hint">⚠️ {t.share_panel_firewall_hint}</div>

          <div className="aiterm-share__viewers">
            <div className="aiterm-share__label">
              {t.share_panel_viewers}（{viewers.length}）
            </div>
            {viewers.length === 0 && (
              <div className="aiterm-share__empty">{t.share_panel_no_viewers}</div>
            )}
            {viewers.map((v) => (
              <div key={v.viewerId} className="aiterm-share__viewer">
                <span className="aiterm-share__viewer-name">{v.displayName}</span>
                <span className="aiterm-share__viewer-mode">
                  {v.mode === "control" ? t.share_panel_mode_control : t.share_panel_mode_read_only}
                </span>
                {v.mode === "control" && (
                  <button
                    className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                    onClick={() => void revokeControl()}
                  >
                    {t.share_panel_revoke_control}
                  </button>
                )}
                <button
                  className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                  onClick={() => void kick(v.viewerId)}
                >
                  {t.share_panel_kick}
                </button>
              </div>
            ))}
          </div>

          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm aiterm-share__stop"
            onClick={() => {
              void stop();
              setOpen(false);
            }}
          >
            {t.share_panel_stop}
          </button>
        </div>
      )}
    </span>
  );
}
