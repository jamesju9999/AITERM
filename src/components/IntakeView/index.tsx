import { useCallback, useEffect, useState } from "react";
import {
  intakeApprove,
  intakeAttachmentData,
  intakeAttachments,
  intakeDeleteSender,
  intakeDropped,
  intakeGetConfig,
  intakeList,
  intakeReject,
  intakeSendReply,
  intakeSenders,
  intakeSetConfig,
  intakeUpdateDraft,
  intakeUpsertSender,
  type IntakeAttachment,
  type IntakeConfig,
  type IntakeItem,
  type IntakeSender,
  type IntakeStatus,
  type Level,
} from "../../ipc/intake";
import { MailIcon } from "../Icons";
import { mailListAccounts, type MailAccount } from "../../ipc/mail";
import { useIntakeChanged } from "../../hooks/useIntakeChanged";
import { useLocale } from "../../contexts/LocaleContext";
import { validAddress } from "../../lib/intakeAddress";
import "./index.css";

const STATUSES: IntakeStatus[] = ["pending", "queued", "running", "draft", "done", "rejected"];

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function IntakeView() {
  const { t } = useLocale();
  const [items, setItems] = useState<IntakeItem[]>([]);
  const [filter, setFilter] = useState<IntakeStatus | "all">("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(() => {
    intakeList(filter === "all" ? undefined : filter)
      .then(setItems)
      .catch((e) => setError(errText(e)));
  }, [filter]);

  useEffect(reload, [reload]);
  useIntakeChanged(reload);

  const selected = items.find((i) => i.id === selectedId) ?? null;

  async function run(fn: () => Promise<unknown>) {
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errText(e));
    }
    reload();
  }

  return (
    <div className="intake">
      <div className="intake__bar">
        <h2 className="intake__title"><MailIcon size={20} className="intake__title-icon" />{t.intake_tab}</h2>
        <select
          aria-label={t.intake_filter_label}
          value={filter}
          onChange={(e) => setFilter(e.target.value as IntakeStatus | "all")}
        >
          <option value="all">{t.intake_filter_all}</option>
          {STATUSES.map((s) => (
            <option key={s} value={s}>
              {t[`intake_status_${s}` as const]}
            </option>
          ))}
        </select>
        <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => setShowSettings((v) => !v)}>
          {t.intake_settings}
        </button>
      </div>
      {error && <div className="intake__error" role="alert">{error}</div>}

      {showSettings ? (
        <SettingsPanel onError={setError} />
      ) : (
        <div className="intake__body">
          <ul className="intake__list" aria-label={t.intake_tab}>
            {items.length === 0 && <li className="intake__empty">{t.intake_empty}</li>}
            {items.map((i) => (
              <li key={i.id}>
                <button
                  className={`intake__row ${i.id === selectedId ? "intake__row--active" : ""}`}
                  onClick={() => setSelectedId(i.id)}
                >
                  <span className={`intake__badge intake__badge--${i.status}`}>{t[`intake_status_${i.status}` as const]}</span>
                  <span className="intake__subject">{i.subject || t.intake_no_subject}</span>
                  <span className="intake__sender">{t.intake_asker}：{i.sender_name ? `${i.sender_name} <${i.sender}>` : i.sender} · {i.sender_level === "internal" ? t.intake_level_internal : t.intake_level_external}</span>
                </button>
              </li>
            ))}
          </ul>
          <div className="intake__detail">
            {selected ? <Detail key={selected.id} item={selected} run={run} /> : <p className="intake__empty">{t.intake_pick}</p>}
          </div>
        </div>
      )}
    </div>
  );
}

function Attachments({ itemId }: { itemId: string }) {
  const { t } = useLocale();
  const [atts, setAtts] = useState<IntakeAttachment[]>([]);
  const [urls, setUrls] = useState<Record<string, string>>({});
  const [failed, setFailed] = useState<Record<string, true>>({});

  useEffect(() => {
    let alive = true;
    intakeAttachments(itemId)
      .then(async (list) => {
        if (!alive) return;
        setAtts(list);
        for (const a of list) {
          intakeAttachmentData(a.id)
            .then((u) => alive && setUrls((m) => ({ ...m, [a.id]: u })))
            .catch(() => alive && setFailed((m) => ({ ...m, [a.id]: true })));
        }
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [itemId]);

  if (atts.length === 0) return null;
  return (
    <section>
      <h4>{t.intake_attachments.replace("{n}", String(atts.length))}</h4>
      <div className="intake__images">
        {atts.map((a) =>
          urls[a.id] ? (
            <a key={a.id} href={urls[a.id]} target="_blank" rel="noreferrer" title={a.filename}>
              <img src={urls[a.id]} alt={a.filename} className="intake__thumb" />
            </a>
          ) : (
            <span key={a.id} className="intake__thumb intake__thumb--empty">
              {failed[a.id] ? t.intake_attachment_failed : a.filename}
            </span>
          ),
        )}
      </div>
    </section>
  );
}

function Detail({ item, run }: { item: IntakeItem; run: (fn: () => Promise<unknown>) => Promise<void> }) {
  const { t } = useLocale();
  const [draft, setDraft] = useState(item.reply_draft ?? "");
  return (
    <div>
      <h3>{item.subject || t.intake_no_subject}</h3>
      <p className="intake__asker">
        <span className="intake__asker-label">{t.intake_asker}</span>
        {item.sender_name && <strong>{item.sender_name}</strong>}
        {item.sender_name ? <span className="intake__meta">{item.sender}</span> : <strong>{item.sender}</strong>}
        <span className={`intake__level intake__level--${item.sender_level}`}>
          {item.sender_level === "internal" ? t.intake_level_internal : t.intake_level_external}
        </span>
        <span className="intake__meta">{item.source}</span>
      </p>
      {item.kind && (
        <p className="intake__meta">{item.kind === "code_change" ? t.intake_kind_code_change : t.intake_kind_question}</p>
      )}
      {item.error && <p className="intake__error">{item.error}</p>}
      <pre className="intake__text">{item.body}</pre>
      <Attachments itemId={item.id} />
      {item.verification && (
        <section>
          <h4 className={item.verified ? "intake__ok" : "intake__error"}>
            {item.verified ? t.intake_verify_passed : t.intake_verify_failed}
          </h4>
          <pre className="intake__text">{item.verification}</pre>
        </section>
      )}
      <div className="intake__actions">
        {item.status === "pending" && (
          <>
            <button className="aiterm-btn aiterm-btn--primary aiterm-btn--sm" onClick={() => void run(() => intakeApprove(item.id))}>
              {t.intake_action_approve}
            </button>
            <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void run(() => intakeReject(item.id))}>
              {t.intake_action_reject}
            </button>
          </>
        )}
        {item.status === "queued" && (
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void run(() => intakeReject(item.id))}>
            {t.intake_action_reject}
          </button>
        )}
      </div>
      {(item.status === "draft" || item.status === "done") && item.reply_draft !== null && (
        <section>
          <h4>{t.intake_reply}</h4>
          <textarea
            className="intake__draft"
            value={draft}
            readOnly={item.status === "done"}
            onChange={(e) => setDraft(e.target.value)}
            rows={10}
          />
          {item.status === "draft" && (
            <div className="intake__actions">
              <button
                className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
                onClick={() =>
                  void run(async () => {
                    await intakeUpdateDraft(item.id, draft);
                    await intakeSendReply(item.id);
                  })
                }
              >
                {t.intake_action_send}
              </button>
              <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void run(() => intakeUpdateDraft(item.id, draft))}>
                {t.intake_action_save_draft}
              </button>
            </div>
          )}
        </section>
      )}
    </div>
  );
}

function SettingsPanel({ onError }: { onError: (m: string | null) => void }) {
  const { t } = useLocale();
  const [cfg, setCfg] = useState<IntakeConfig | null>(null);
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  const [senders, setSenders] = useState<IntakeSender[]>([]);
  const [dropped, setDropped] = useState<[string, number][]>([]);
  const [newAddr, setNewAddr] = useState("");
  const [newLevel, setNewLevel] = useState<Level>("internal");
  const [saved, setSaved] = useState(false);

  const loadSenders = useCallback(() => {
    intakeSenders().then(setSenders).catch((e) => onError(errText(e)));
    intakeDropped().then(setDropped).catch(() => {});
  }, [onError]);

  useEffect(() => {
    intakeGetConfig().then(setCfg).catch((e) => onError(errText(e)));
    mailListAccounts().then(setAccounts).catch(() => {});
    loadSenders();
  }, [loadSenders, onError]);

  if (!cfg) return null;
  const set = <K extends keyof IntakeConfig>(k: K, v: IntakeConfig[K]) => {
    setSaved(false);
    setCfg({ ...cfg, [k]: v });
  };

  async function save() {
    try {
      await intakeSetConfig(cfg!);
      setSaved(true);
      onError(null);
    } catch (e) {
      onError(errText(e));
    }
  }

  async function addSender() {
    try {
      await intakeUpsertSender({ address: newAddr.trim(), level: newLevel, skip_auth: false });
      setNewAddr("");
      onError(null);
      loadSenders();
    } catch (e) {
      onError(errText(e));
    }
  }

  const needsAlias = cfg.mode === "alias" || cfg.mode === "alias_or_tag";
  const needsTag = cfg.mode === "subject_tag" || cfg.mode === "alias_or_tag";

  return (
    <div className="intake__settings">
      <section className="intake__card">
        <h3 className="intake__card-title">{t.intake_sec_general}</h3>
      <label className="intake__field">
        <input type="checkbox" checked={cfg.enabled} onChange={(e) => set("enabled", e.target.checked)} />
        {t.intake_set_enabled}
      </label>
      </section>
      <section className="intake__card">
        <h3 className="intake__card-title">{t.intake_sec_channels}</h3>
      <label className="intake__field">
        <input type="checkbox" checked={cfg.telegram_enabled} onChange={(e) => set("telegram_enabled", e.target.checked)} />
        {t.intake_set_telegram}
      </label>
      <p className="intake__meta">{t.intake_set_telegram_hint}</p>
      <label className="intake__field">
        {t.intake_set_web}
        <select value={cfg.web_mode} onChange={(e) => set("web_mode", e.target.value as IntakeConfig["web_mode"])}>
          <option value="off">{t.intake_web_off}</option>
          <option value="local">{t.intake_web_local}</option>
          <option value="lan">{t.intake_web_lan}</option>
        </select>
        {cfg.web_mode !== "off" && (
          <input
            type="number"
            aria-label={t.intake_set_web_port}
            min={1024}
            max={65535}
            value={cfg.web_port}
            onChange={(e) => set("web_port", Math.min(65535, Math.max(1024, Number(e.target.value) || 8320)))}
          />
        )}
      </label>
      {cfg.web_mode !== "off" && (
        <p className="intake__meta">
          {t.intake_web_url}：http://{cfg.web_mode === "lan" ? "<本機 IP>" : "127.0.0.1"}:{cfg.web_port}/
        </p>
      )}
      {cfg.web_mode === "lan" && <p className="intake__error">{t.intake_web_lan_warning}</p>}
      <label className="intake__field">
        {t.intake_set_account}
        <select value={cfg.account_id} onChange={(e) => set("account_id", e.target.value)}>
          <option value="">—</option>
          {accounts.map((a) => (
            <option key={a.id} value={a.id}>{a.email}</option>
          ))}
        </select>
      </label>
      <label className="intake__field">
        {t.intake_set_mode}
        <select value={cfg.mode} onChange={(e) => set("mode", e.target.value as IntakeConfig["mode"])}>
          <option value="alias_or_tag">{t.intake_mode_alias_or_tag}</option>
          <option value="alias">{t.intake_mode_alias}</option>
          <option value="subject_tag">{t.intake_mode_subject_tag}</option>
          <option value="dedicated">{t.intake_mode_dedicated}</option>
        </select>
      </label>
      {needsAlias && (
        <label className="intake__field">
          {t.intake_set_alias}
          <input value={cfg.alias} placeholder="you+aiterm@gmail.com" onChange={(e) => set("alias", e.target.value)} />
        </label>
      )}
      {needsTag && (
        <label className="intake__field">
          {t.intake_set_tag}
          <input value={cfg.tag} placeholder="[AITerm]" onChange={(e) => set("tag", e.target.value)} />
        </label>
      )}
      <label className="intake__field">
        {t.intake_set_auth_server}
        <input value={cfg.auth_server} onChange={(e) => set("auth_server", e.target.value)} />
      </label>
      </section>
      <section className="intake__card">
        <h3 className="intake__card-title">{t.intake_sec_flow}</h3>
      {(["internal", "external"] as const).map((lv) => (
        <div key={lv} className="intake__field intake__field--row">
          <span>{lv === "internal" ? t.intake_level_internal : t.intake_level_external}</span>
          <select
            aria-label={`${lv}-dispatch`}
            value={cfg[`dispatch_${lv}`]}
            onChange={(e) => set(`dispatch_${lv}`, e.target.value as "auto" | "manual")}
          >
            <option value="auto">{t.intake_dispatch_auto}</option>
            <option value="manual">{t.intake_dispatch_manual}</option>
          </select>
          <label>
            <input
              type="checkbox"
              checked={cfg[`reply_${lv}_auto`]}
              onChange={(e) => set(`reply_${lv}_auto`, e.target.checked)}
            />
            {t.intake_reply_auto}
          </label>
        </div>
      ))}
      <label className="intake__field">
        {t.intake_set_rate}
        <input
          type="number"
          min={0}
          value={cfg.rate_limit_per_hour}
          onChange={(e) => set("rate_limit_per_hour", Math.max(0, Number(e.target.value) || 0))}
        />
      </label>
      </section>
      <section className="intake__card">
        <h3 className="intake__card-title">{t.intake_sec_verify}</h3>
      <label className="intake__field">
        {t.intake_set_verify_command}
        <input
          value={cfg.verify_command}
          placeholder="npm test"
          onChange={(e) => set("verify_command", e.target.value)}
        />
      </label>
      <label className="intake__field">
        {t.intake_set_verify_timeout}
        <input
          type="number"
          min={1}
          value={cfg.verify_timeout_secs}
          onChange={(e) => set("verify_timeout_secs", Math.max(1, Number(e.target.value) || 1))}
        />
      </label>
      </section>

      <section className="intake__card">
      <h3 className="intake__card-title">{t.intake_senders}</h3>
      <ul className="intake__senders">
        {senders.map((s) => (
          <li key={s.address}>
            {s.address} · {s.level === "internal" ? t.intake_level_internal : t.intake_level_external}
            {s.level === "external" && (
              <label>
                <input
                  type="checkbox"
                  checked={s.skip_auth}
                  onChange={(e) => void intakeUpsertSender({ ...s, skip_auth: e.target.checked }).then(loadSenders)}
                />
                {t.intake_skip_auth}
              </label>
            )}
            <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void intakeDeleteSender(s.address).then(loadSenders)}>
              {t.intake_remove}
            </button>
          </li>
        ))}
      </ul>
      <div className="intake__field intake__field--inline">
        <input aria-label={t.intake_sender_address} value={newAddr} placeholder="name@example.com / tg:123456" onChange={(e) => setNewAddr(e.target.value)} />
        <select aria-label={t.intake_sender_level} value={newLevel} onChange={(e) => setNewLevel(e.target.value as Level)}>
          <option value="internal">{t.intake_level_internal}</option>
          <option value="external">{t.intake_level_external}</option>
        </select>
        <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={!validAddress(newAddr)} onClick={() => void addSender()}>
          {t.intake_add}
        </button>
      </div>
      {dropped.length > 0 && (
        <p className="intake__meta">
          {t.intake_dropped}：{dropped.map(([r, n]) => `${r} ${n}`).join("、")}
        </p>
      )}
      </section>
      <div className="intake__actions intake__save">
        <button className="aiterm-btn aiterm-btn--primary" onClick={() => void save()}>
          {t.intake_set_save}
        </button>
        {saved && <span className="intake__saved" role="status">✓ {t.intake_set_saved}</span>}
      </div>
    </div>
  );
}
