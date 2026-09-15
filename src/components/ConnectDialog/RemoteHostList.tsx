import { useState } from "react";
import type { RemoteHostInfo } from "../../ipc/remoteHosts";
import { useLocale } from "../../contexts/LocaleContext";

interface Props {
  hosts: RemoteHostInfo[];
  onConnect: (host: RemoteHostInfo) => void;
  onEdit: (host: RemoteHostInfo) => void;
  /** **只在使用者按下確認列的「刪除」之後才會被呼叫。** 刪除會連 keychain
   *  的金鑰一起刪掉，一按就生效太危險，所以確認這一步由清單自己守住——
   *  破壞性的動作由擁有那一列的元件把關。回傳 Promise 時會等它完成才收起
   *  確認列，跟這一步搬進來之前的時序一致。 */
  onDelete: (host: RemoteHostInfo) => void | Promise<void>;
  /** 有連線正在進行中時整排都要停用——不然使用者可以在手動連線送出後、
   *  結果還沒回來之前，再點一筆已存主機，兩個 `shareViewerConnect` 同時飛
   *  出去。跟 `ConnectDialog` 送出鈕的 `disabled={busy || ...}` 是同一條規則。 */
  disabled?: boolean;
}

/**
 * 地址簿清單。
 *
 * 從 `ConnectDialog` 抽出來：那個元件原本已經同時負責模式切換、mDNS 搜尋與
 * 送出，再加上清單管理會變成四件事。
 *
 * 沒有任何條目時整塊不渲染——空清單的標題對沒用過這個功能的人只是雜訊。
 */
export function RemoteHostList({ hosts, onConnect, onEdit, onDelete, disabled = false }: Props) {
  const { t } = useLocale();
  /** 等待確認刪除的那一筆。確認列用就地渲染，**不用 `window.confirm`**：
   *  這個 repo 被原生對話框咬過（StrictMode 雙呼叫會開兩個、第一個的結果
   *  被丟掉導致卡死）。 */
  const [confirming, setConfirming] = useState<RemoteHostInfo | null>(null);

  async function confirmDelete(h: RemoteHostInfo) {
    await onDelete(h);
    setConfirming(null);
  }

  if (hosts.length === 0) return null;
  return (
    <div className="aiterm-connect__saved">
      <div className="aiterm-connect__label">{t.connect_saved_title}</div>
      <ul className="aiterm-connect__saved-list" role="list">
        {hosts.map((h) => (
          <li key={h.id} className="aiterm-connect__saved-row">
            <button
              type="button"
              className="aiterm-connect__saved-main"
              onClick={() => onConnect(h)}
              title={t.connect_saved_connect}
              disabled={disabled}
            >
              <span className="aiterm-connect__saved-name">{h.name}</span>
              <span className="aiterm-connect__saved-addr">{`${h.host}:${h.port}`}</span>
              {/* 金鑰不在本機時先說清楚，不要等連線失敗才讓使用者猜。 */}
              {!h.has_key && (
                <span className="aiterm-connect__saved-warn" data-testid="no-key">
                  {t.connect_saved_no_key}
                </span>
              )}
            </button>
            <button
              type="button"
              className="aiterm-connect__saved-action"
              onClick={() => onEdit(h)}
              disabled={disabled}
            >
              {t.connect_saved_edit}
            </button>
            <button
              type="button"
              className="aiterm-connect__saved-action"
              onClick={() => setConfirming(h)}
              disabled={disabled}
            >
              {t.connect_saved_delete}
            </button>
          </li>
        ))}
      </ul>

      {/* 放在 </ul> 之後而不是列裡：清單有 max-height 會捲動，確認列放在捲動
          區裡可能被捲出視野。 */}
      {confirming && (
        <div className="aiterm-connect__confirm">
          <div className="aiterm-connect__panel-text">
            {t.connect_saved_delete_confirm.replace("{name}", confirming.name)}
          </div>
          <div className="aiterm-connect__actions">
            <button
              className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
              onClick={() => setConfirming(null)}
            >
              {t.connect_cancel}
            </button>
            <button
              className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
              onClick={() => void confirmDelete(confirming)}
            >
              {t.connect_saved_delete}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
