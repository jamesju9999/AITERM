import { useEffect, useState } from "react";
import { shareDiscover } from "../../ipc/share";
import { shareViewerConnect } from "../../ipc/shareViewer";
import {
  ERR_KEYCHAIN_UNAVAILABLE,
  ERR_SAVED_KEY_MISSING,
  remoteHostsAdd,
  remoteHostsList,
  remoteHostsRemove,
  remoteHostsUpdate,
  type RemoteHostInfo,
} from "../../ipc/remoteHosts";
import { RemoteHostList } from "./RemoteHostList";
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
 *
 * 地址簿（`RemoteHostList`）讓使用者不必每次都重貼金鑰：點一筆已存的條目
 * 只送 `savedHostId`，金鑰由後端自己去 keychain 取，前端從頭到尾拿不到它。
 */
export function ConnectDialog({ onConnected, onCancel }: Props) {
  const { t } = useLocale();
  const [code, setCode] = useState("");
  const [name, setName] = useState("");
  const [manualOpen, setManualOpen] = useState(false);
  const [address, setAddress] = useState("");
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [searching, setSearching] = useState(false);

  const [hosts, setHosts] = useState<RemoteHostInfo[]>([]);
  /** 連上之後、還沒決定要不要存的那筆。null 代表沒有待決定的。 */
  const [pendingSave, setPendingSave] = useState<{
    host: string;
    port: number;
    secret: string;
    connId: string;
    sas: string;
    label: string;
  } | null>(null);
  const [saveName, setSaveName] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  /** 等待確認刪除的那一筆。刪除會連 keychain 的金鑰一起刪掉，不能一按就生效。 */
  const [confirmDelete, setConfirmDelete] = useState<RemoteHostInfo | null>(null);

  useEffect(() => {
    // 地址簿只是連線對話框的一個輔助入口，不是使用者非用不可的路徑——手動
    // 輸入永遠走得通。抓不到清單就安靜顯示空清單即可，不值得為了這件事跳出
    // 一個會擋住整個對話框的錯誤（跟 VcsConnectionsPage 的既有慣例一致）。
    void remoteHostsList()
      .then(setHosts)
      .catch((e) => {
        console.error(e);
        setHosts([]);
      });
  }, []);

  // 金鑰模式：位址與金鑰都填了。這種連線**沒有短碼**——身分完全由金鑰決定，
  // 觀看端連送出去的 `code` 都是空字串（見 `share::viewer` 的 `Join`）。所以
  // 送出鈕不能再要求短碼滿 6 位，否則金鑰填好了按鈕還是灰的，整條 CLI host
  // 的路走不通。
  //
  // 反過來也要守住：沒填金鑰時，短碼仍然是必填——放寬成「有位址就能按」會讓
  // 短碼模式在碼還沒打完時就送出去。
  const keyMode = manualOpen && address.trim() !== "" && key.trim() !== "";
  // 編輯一筆已存的條目時，金鑰留空代表「不改金鑰」（後端把空字串當成
  // no-op）——所以編輯必須光靠位址就能送出，不能死守 keyMode 那套「一定要
  // 打金鑰」的規則，否則使用者連改個別名都做不到：送出鈕會永遠停在灰色。
  const editMode = manualOpen && editingId !== null && address.trim() !== "";

  async function connectTo(
    host: string,
    port: number,
    addressLabel: string,
    opts: { savedHostId?: string; typedKey?: string } = {},
  ) {
    try {
      const { connId, sas } = await shareViewerConnect({
        host,
        port,
        code,
        displayName: name || "AITerm",
        // 空字串要送 undefined，不能送 ""。後端把 Some("") 當成「有金鑰但是空的」，
        // 握手會失敗，而錯誤訊息會指向金鑰不符——對一個根本沒填金鑰的使用者來說
        // 完全誤導。
        key: opts.savedHostId ? undefined : opts.typedKey,
        savedHostId: opts.savedHostId,
      });
      // 只有「手動輸入的金鑰模式」或「正在編輯一筆已存條目」才問要不要存：
      // 短碼沒有固定金鑰可存，從地址簿來的本來就存過了。
      //
      // **`|| editingId` 不能省。** 若只看 `opts.typedKey`，編輯時只要使用者
      // 沒有重新輸入金鑰（正常做法——只是改個別名或位址），這裡就會直接跳到
      // `onConnected`：使用者的修改被靜默丟棄，而且 `editingId` 永遠不會被
      // `finishPending` 清掉，留到下一次真的新增時把它誤當成更新，覆蓋掉
      // 不相干的舊條目。
      if (!opts.savedHostId && (opts.typedKey || editingId)) {
        setPendingSave({
          host,
          port,
          secret: opts.typedKey ?? "",
          connId,
          sas,
          label: addressLabel,
        });
        return;
      }
      // **不能省略。** 這條是 `finishPending` 以外唯一一個會走到
      // `onConnected` 的出口（短碼模式、或點地址簿裡的另一筆）。如果使用者
      // 先按了「編輯」（`editingId` 被設成某筆的 id），還沒送出手動表單就
      // 改點清單裡別的已存主機連線，這裡如果不清掉，`editingId` 會一路
      // 殘留到下一次完全無關的手動新增，把它誤當成更新，覆蓋掉那一筆。
      setEditingId(null);
      onConnected(connId, sas, addressLabel);
    } catch (e) {
      const msg = String(e);
      // **keychain 讀不到要先判，而且用 includes。** 後端送的是
      // `remote_host_keychain_unavailable: <底層原因>`。這種情況重貼金鑰沒有
      // 任何用處——金鑰其實好好的，是金鑰圈打不開，所以不要展開手動欄位叫
      // 使用者重輸入。
      if (msg.includes(ERR_KEYCHAIN_UNAVAILABLE)) {
        setError(t.connect_keychain_unavailable);
        return;
      }
      if (msg.includes(ERR_SAVED_KEY_MISSING)) {
        // 金鑰不在這台電腦上：把位址帶進手動欄位，使用者只要重貼金鑰。
        // 絕對不能靜默改用短碼模式重試——那會得到一個指向短碼的錯誤訊息。
        setManualOpen(true);
        setAddress(addressLabel);
        setKey("");
        setError(t.connect_saved_no_key);
        return;
      }
      // 連不上要說原因，不要靜默關閉——使用者才知道下一步該做什麼。
      setError(t.connect_failed.replace("{error}", msg));
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
      await connectTo(parsed.host, parsed.port, address, {
        typedKey: key.trim() === "" ? undefined : key.trim(),
      });
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

  async function refresh() {
    setHosts(await remoteHostsList());
  }

  async function removeHost(h: RemoteHostInfo) {
    await remoteHostsRemove(h.id);
    setConfirmDelete(null);
    await refresh();
  }

  async function editHost(h: RemoteHostInfo) {
    // 編輯前先重抓一次。`ConfigStore::update_remote_host` 對已經不存在的 id
    // 是靜默 no-op（跟 vcs.rs 原本的行為一樣）——如果直接拿畫面上可能已經在
    // 別台裝置被刪掉的那一筆送出，`remote_hosts_update` 會回 Ok 但設定檔完全
    // 沒變，金鑰卻已經被寫進 keychain，留下一個永遠用不到的孤兒條目。
    const latest = await remoteHostsList();
    setHosts(latest);
    const current = latest.find((x) => x.id === h.id);
    if (!current) {
      setError(t.connect_saved_missing);
      return;
    }
    // 編輯＝把這一筆帶進手動欄位，金鑰留空（前端拿不到已存的金鑰）。
    // 使用者只改別名時送空 secret，後端會當成「不改金鑰」。
    setManualOpen(true);
    setAddress(`${current.host}:${current.port}`);
    setKey("");
    setEditingId(current.id);
  }

  async function confirmSave() {
    if (!pendingSave) return;
    if (editingId) {
      await remoteHostsUpdate({
        id: editingId,
        name: saveName || pendingSave.label,
        host: pendingSave.host,
        port: pendingSave.port,
        secret: pendingSave.secret,
      });
    } else {
      await remoteHostsAdd({
        name: saveName || pendingSave.label,
        host: pendingSave.host,
        port: pendingSave.port,
        secret: pendingSave.secret,
      });
    }
    await refresh();
    finishPending();
  }

  function finishPending() {
    if (!pendingSave) return;
    const p = pendingSave;
    setPendingSave(null);
    setSaveName("");
    setEditingId(null);
    onConnected(p.connId, p.sas, p.label);
  }

  return (
    <div className="aiterm-connect__backdrop">
      <div className="aiterm-connect" role="dialog" aria-modal="true">
        <div className="aiterm-connect__title">{t.connect_title}</div>

        <RemoteHostList
          hosts={hosts}
          disabled={busy}
          onConnect={(h) =>
            void connectTo(h.host, h.port, `${h.host}:${h.port}`, { savedHostId: h.id })
          }
          onEdit={editHost}
          onDelete={setConfirmDelete}
        />

        {confirmDelete && (
          <div className="aiterm-connect__confirm">
            <div>{t.connect_saved_delete_confirm.replace("{name}", confirmDelete.name)}</div>
            <div className="aiterm-connect__actions">
              <button
                className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                onClick={() => setConfirmDelete(null)}
              >
                {t.connect_cancel}
              </button>
              <button
                className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                onClick={() => void removeHost(confirmDelete)}
              >
                {t.connect_saved_delete}
              </button>
            </div>
          </div>
        )}

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

            <label className="aiterm-connect__label" htmlFor="aiterm-connect-key">
              {t.connect_key_label}
            </label>
            <input
              id="aiterm-connect-key"
              className="aiterm-connect__text"
              type="text"
              value={key}
              onChange={(e) => setKey(e.target.value)}
              autoComplete="off"
              spellCheck={false}
            />
            <p className="aiterm-connect__hint">{t.connect_key_hint}</p>
          </>
        )}

        {searching && <div className="aiterm-connect__searching">{t.connect_searching}</div>}

        {error && <div className="aiterm-connect__error">{error}</div>}

        {pendingSave && (
          <div className="aiterm-connect__save">
            <div>{t.connect_save_prompt}</div>
            <label className="aiterm-connect__label" htmlFor="aiterm-connect-savename">
              {t.connect_save_name_label}
            </label>
            <input
              id="aiterm-connect-savename"
              className="aiterm-connect__text"
              type="text"
              value={saveName}
              onChange={(e) => setSaveName(e.target.value)}
            />
            <div className="aiterm-connect__actions">
              <button
                className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                onClick={finishPending}
              >
                {t.connect_save_skip}
              </button>
              <button
                className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                onClick={() => void confirmSave()}
              >
                {t.connect_save_confirm}
              </button>
            </div>
          </div>
        )}

        <div className="aiterm-connect__actions">
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={onCancel}>
            {t.connect_cancel}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
            disabled={busy || (!keyMode && !editMode && code.length !== 6)}
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
