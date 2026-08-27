import { useState } from "react";
import { shareDiscover } from "../../ipc/share";
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
 * **手動位址永遠是主路徑**（見 spec 的決策紀錄）：mDNS 在公司網路／跨
 * VLAN／訪客 Wi-Fi 常常失效，所以手動那條路必須一直走得通——`submit()`
 * 一旦偵測到手動欄位有內容就完全跳過 mDNS，直接用它連。
 *
 * 平常把手動欄位收起來，只有 mDNS 查無結果或結果有歧義（多台機器用了
 * 同一組短碼）時才自動展開，並依情境顯示不同文案。
 */
export function ConnectDialog({ onConnected, onCancel }: Props) {
  const { t } = useLocale();
  const [code, setCode] = useState("");
  const [name, setName] = useState("");
  const [manualOpen, setManualOpen] = useState(false);
  const [address, setAddress] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [searching, setSearching] = useState(false);

  async function connectTo(host: string, port: number, addressLabel: string) {
    try {
      const { connId, sas } = await shareViewerConnect(host, port, code, name || "AITerm");
      onConnected(connId, sas, addressLabel);
    } catch (e) {
      // 連不上要說原因，不要靜默關閉——使用者才知道下一步該做什麼。
      setError(t.connect_failed.replace("{error}", String(e)));
    }
  }

  async function submit() {
    setError(null);

    // 手動位址欄位已經展開且有填：永遠優先，完全不跑 mDNS 查找。使用者
    // 已經知道要連哪裡，不該被搜尋卡住或蓋掉他輸入的內容。
    if (manualOpen && address.trim()) {
      const parsed = parseAddress(address);
      if (!parsed) {
        setError(t.connect_bad_address);
        return;
      }
      setBusy(true);
      await connectTo(parsed.host, parsed.port, address);
      setBusy(false);
      return;
    }

    setBusy(true);
    setSearching(true);
    try {
      const result = await shareDiscover(code);
      if (result.kind === "found") {
        const label = `${result.host}:${result.port}`;
        await connectTo(result.host, result.port, label);
        return;
      }
      setManualOpen(true);
      setError(result.kind === "ambiguous" ? t.connect_ambiguous : t.connect_not_found);
    } finally {
      setSearching(false);
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

        {searching && <div className="aiterm-connect__searching">{t.connect_searching}</div>}

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
