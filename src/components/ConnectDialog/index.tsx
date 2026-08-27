import { useState } from "react";
import { shareViewerConnect } from "../../ipc/shareViewer";
import { useLocale } from "../../contexts/LocaleContext";
import "./index.css";

interface Props {
  /** 連上之後回報連線 id、這一端算出的驗證碼、以及對方位址，讓上層開一個
   *  `remote-terminal` 分頁。SAS 跟著連線回傳值走而不是事件——見
   *  `shareViewerConnect` 的說明。 */
  onConnected: (connId: string, sas: string, hostLabel: string) => void;
  onCancel: () => void;
}

/**
 * 觀看端的連線入口。
 *
 * **手動位址永遠是主路徑**（見 spec 的決策紀錄）：2C 會加上 mDNS 自動發現，
 * 但它在公司網路／跨 VLAN／訪客 Wi-Fi 常常失效，所以手動那條路必須一直
 * 走得通。這個階段 mDNS 還沒上線，所以手動是唯一的路。
 *
 * 平常把手動欄位收起來、需要時展開——spec 選的是「找不到時自動展開並說明
 * 原因」，2C 接上 mDNS 後會補上自動展開那段。
 */
export function ConnectDialog({ onConnected, onCancel }: Props) {
  const { t } = useLocale();
  const [code, setCode] = useState("");
  const [name, setName] = useState("");
  const [manualOpen, setManualOpen] = useState(false);
  const [address, setAddress] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit() {
    setError(null);
    const parsed = parseAddress(address);
    if (!parsed) {
      setError(t.connect_bad_address);
      return;
    }
    setBusy(true);
    try {
      const { connId, sas } = await shareViewerConnect(
        parsed.host,
        parsed.port,
        code,
        name || "AITerm",
      );
      onConnected(connId, sas, address);
    } catch (e) {
      // 連不上要說原因，不要靜默關閉——使用者才知道下一步該做什麼。
      setError(t.connect_failed.replace("{error}", String(e)));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="aiterm-connect__backdrop">
      <div className="aiterm-connect" role="dialog" aria-modal="true">
        <div className="aiterm-connect__title">{t.connect_title}</div>

        <label className="aiterm-connect__label" htmlFor="aiterm-connect-code">
          {t.connect_code_label}
        </label>
        <input
          id="aiterm-connect-code"
          className="aiterm-connect__code"
          type="text"
          inputMode="numeric"
          maxLength={6}
          autoFocus
          value={code}
          onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
        />

        <label className="aiterm-connect__label" htmlFor="aiterm-connect-name">
          {t.connect_name_label}
        </label>
        <input
          id="aiterm-connect-name"
          className="aiterm-connect__text"
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
        />

        {!manualOpen && (
          <button className="aiterm-connect__toggle" onClick={() => setManualOpen(true)}>
            ▸ {t.connect_manual_toggle}
          </button>
        )}

        {manualOpen && (
          <>
            <label className="aiterm-connect__label" htmlFor="aiterm-connect-addr">
              {t.connect_manual_label}
            </label>
            <input
              id="aiterm-connect-addr"
              className="aiterm-connect__text"
              type="text"
              placeholder={t.connect_manual_placeholder}
              value={address}
              onChange={(e) => setAddress(e.target.value)}
            />
          </>
        )}

        {error && <div className="aiterm-connect__error">{error}</div>}

        <div className="aiterm-connect__actions">
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onCancel}>
            {t.connect_cancel}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            disabled={busy || code.length !== 6}
            onClick={() => void submit()}
          >
            {t.connect_submit}
          </button>
        </div>
      </div>
    </div>
  );
}

/** `host:port` → `{ host, port }`。格式不對回 `null`。 */
function parseAddress(raw: string): { host: string; port: number } | null {
  const m = raw.trim().match(/^(.+):(\d{1,5})$/);
  if (!m) return null;
  const port = Number(m[2]);
  if (!Number.isInteger(port) || port < 1 || port > 65535) return null;
  return { host: m[1], port };
}
