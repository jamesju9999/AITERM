import type { RemoteHostInfo } from "../../ipc/remoteHosts";
import { useLocale } from "../../contexts/LocaleContext";

interface Props {
  hosts: RemoteHostInfo[];
  onConnect: (host: RemoteHostInfo) => void;
  onEdit: (host: RemoteHostInfo) => void;
  onDelete: (host: RemoteHostInfo) => void;
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
              onClick={() => onDelete(h)}
              disabled={disabled}
            >
              {t.connect_saved_delete}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
