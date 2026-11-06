import { useCallback, useEffect, useState } from "react";
import {
  intakeAdoptLegacy,
  intakeApprove,
  intakeAssign,
  intakeCopyLegacyConfig,
  intakeAttachmentData,
  intakeAttachmentText,
  intakeAttachments,
  intakeDelete,
  intakeDeleteSender,
  intakeDropped,
  intakeGetConfig,
  intakeList,
  intakeProjects,
  intakeRefresh,
  intakeReject,
  intakeSendReply,
  intakeSenders,
  intakeSetConfig,
  intakeUpdateDraft,
  intakeUpsertSender,
  type IntakeAttachment,
  type IntakeConfig,
  type IntakeItem,
  type IntakeProject,
  type IntakeSender,
  type IntakeStatus,
  type Level,
} from "../../ipc/intake";
import { MailIcon } from "../Icons";
import { MarkdownText } from "../../lib/markdown";
import { mailListAccounts, type MailAccount } from "../../ipc/mail";
import { useIntakeChanged } from "../../hooks/useIntakeChanged";
import { useLocale } from "../../contexts/LocaleContext";
import { validAddress } from "../../lib/intakeAddress";
import { flowSteps } from "../../lib/intakeFlow";
import "./index.css";

const PROJECT_KEY = "aiterm.intake.project";

function savedProject(): string | null {
  try {
    return localStorage.getItem(PROJECT_KEY);
  } catch {
    return null;
  }
}

const STATUSES: IntakeStatus[] = ["pending", "queued", "running", "draft", "done", "rejected"];

function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function IntakeView() {
  const { t } = useLocale();
  const [projects, setProjects] = useState<IntakeProject[]>([]);
  const [unassigned, setUnassigned] = useState(0);
  const [legacySettings, setLegacySettings] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [choice, setChoice] = useState<string | null>(savedProject);
  const [items, setItems] = useState<IntakeItem[]>([]);
  const [filter, setFilter] = useState<IntakeStatus | "all">("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [refreshMsg, setRefreshMsg] = useState<string | null>(null);

  const loadProjects = useCallback(() => {
    intakeProjects()
      .then((v) => {
        setProjects(v.projects);
        setUnassigned(v.unassigned);
        setLegacySettings(v.legacy_settings);
        setLoaded(true);
      })
      .catch((e) => setError(errText(e)));
  }, []);

  // 選到的專案不存在（被移除、第一次使用）就用第一個專案；「未歸屬」只有在使用者自己選、且還有舊資料時才看。
  const known = choice !== null && (projects.some((p) => p.id === choice) || (choice === "" && unassigned > 0));
  const projectId = known ? choice : (projects[0]?.id ?? "");

  useEffect(() => {
    if (!loaded) return;
    try {
      localStorage.setItem(PROJECT_KEY, projectId);
    } catch {
      /* 沒有儲存空間也能用 */
    }
  }, [loaded, projectId]);

  const reload = useCallback(() => {
    loadProjects();
    intakeList(projectId, filter === "all" ? undefined : filter)
      .then(setItems)
      .catch((e) => setError(errText(e)));
  }, [filter, projectId, loadProjects]);

  useEffect(reload, [reload]);
  useIntakeChanged(reload);

  const selected = items.find((i) => i.id === selectedId) ?? null;
  const current = projects.find((p) => p.id === projectId) ?? null;
  const inUnassigned = projectId === "" && unassigned > 0;

  /** 重新載入清單，並立刻去這個專案的信箱收一次信。 */
  async function refresh() {
    if (refreshing) return;
    setRefreshing(true);
    setRefreshMsg(null);
    setError(null);
    try {
      const r = await intakeRefresh(projectId);
      if (r.error) setError(r.error);
      setRefreshMsg(
        !r.polled ? t.intake_refresh_reloaded : r.new_items > 0 ? t.intake_refresh_new.replace("{n}", String(r.new_items)) : r.ok ? t.intake_refresh_none : null,
      );
    } catch (e) {
      setError(errText(e));
    } finally {
      setRefreshing(false);
      reload();
    }
  }

  async function run(fn: () => Promise<unknown>) {
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errText(e));
    }
    reload();
  }

  const noProject = loaded && projects.length === 0 && unassigned === 0;

  return (
    <div className="intake">
      <div className="intake__bar">
        <h2 className="intake__title"><MailIcon size={20} className="intake__title-icon" />{t.intake_tab}</h2>
        <select
          aria-label={t.intake_project_label}
          className="intake__project-select"
          value={projectId}
          onChange={(e) => {
            setChoice(e.target.value);
            setSelectedId(null);
            setShowSettings(false);
            setError(null);
          }}
        >
          {projects.map((p) => (
            <option key={p.id} value={p.id}>{p.name}{p.open_count > 0 ? ` (${p.open_count})` : ""}</option>
          ))}
          {unassigned > 0 && <option value="">{t.intake_unassigned} ({unassigned})</option>}
        </select>
        {!inUnassigned && !noProject && (
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => setShowSettings((v) => !v)}>
            {t.intake_settings}
          </button>
        )}
      </div>
      {error && <div className="intake__error" role="alert">{error}</div>}

      {noProject ? (
        <p className="intake__empty">{t.intake_no_projects}</p>
      ) : showSettings && !inUnassigned ? (
        <SettingsPanel key={projectId} projectId={projectId} projectName={current?.name ?? ""} legacySettings={legacySettings} onError={setError} />
      ) : (
        <>
          {inUnassigned && <AdoptBar projects={projects} run={run} />}
          <div className="intake__body">
            <div className="intake__listpane">
            <div className="intake__listhead">
              <span className="intake__listtitle">{t.intake_list_title.replace("{n}", String(items.length))}</span>
              {refreshMsg && !refreshing && <span className="intake__refresh-msg" role="status">{refreshMsg}</span>}
              <button
                type="button"
                className={`intake__refresh ${refreshing ? "intake__refresh--busy" : ""}`}
                title={refreshing ? t.intake_refreshing : t.intake_refresh}
                aria-label={t.intake_refresh}
                disabled={refreshing}
                onClick={() => void refresh()}
              >
                ⟳
              </button>
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
            </div>
            <ul className="intake__list" aria-label={t.intake_tab}>
              {items.length === 0 && <li className="intake__empty">{t.intake_empty}</li>}
              {items.map((i) => (
                <li key={i.id} className="intake__li">
                  <button
                    className={`intake__row ${i.id === selectedId ? "intake__row--active" : ""}`}
                    onClick={() => setSelectedId(i.id)}
                  >
                    <span className="intake__rowtop">
                      <span className={`intake__badge intake__badge--${i.status}`}>{t[`intake_status_${i.status}` as const]}</span>
                      <time className="intake__time" dateTime={new Date(i.created_at * 1000).toISOString()}>{formatTime(i.created_at)}</time>
                    </span>
                    <span className="intake__subject">{i.subject || t.intake_no_subject}</span>
                    <span className="intake__sender">{t.intake_asker}：{i.sender_name ? `${i.sender_name} <${i.sender}>` : i.sender} · {i.sender_level === "internal" ? t.intake_level_internal : t.intake_level_external}</span>
                  </button>
                  <DeleteButton item={i} run={run} variant="row" />
                </li>
              ))}
            </ul>
            </div>
            <div className="intake__detail">
              {selected ? <Detail key={selected.id} item={selected} run={run} projects={projects} /> : <p className="intake__empty">{t.intake_pick}</p>}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

/** 「未歸屬」檢視上方：把舊資料一次歸給某個專案。 */
function AdoptBar({ projects, run }: { projects: IntakeProject[]; run: (fn: () => Promise<unknown>) => Promise<void> }) {
  const { t } = useLocale();
  const [target, setTarget] = useState("");
  return (
    <div className="intake__banner intake__adopt">
      <span>{t.intake_adopt_hint}</span>
      <select aria-label={t.intake_assign_pick} value={target} onChange={(e) => setTarget(e.target.value)}>
        <option value="">{t.intake_assign_pick}</option>
        {projects.map((p) => (
          <option key={p.id} value={p.id}>{p.name}</option>
        ))}
      </select>
      <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={!target} onClick={() => void run(() => intakeAdoptLegacy(target))}>
        {t.intake_adopt_all}
      </button>
    </div>
  );
}

function formatSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** 非圖片附件：檔名、大小、複製路徑（貼給別的工具用）；純文字另有預覽。 */
function FileRow({ att }: { att: IntakeAttachment }) {
  const { t } = useLocale();
  const [preview, setPreview] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const isText = att.mime === "text/plain";

  async function togglePreview() {
    if (open) return setOpen(false);
    setOpen(true);
    if (preview === null) {
      try {
        setPreview(await intakeAttachmentText(att.id));
      } catch {
        setPreview("");
        setMsg(t.intake_file_preview_failed);
      }
    }
  }

  async function copyPath() {
    try {
      await navigator.clipboard.writeText(att.path);
      setMsg(t.intake_file_copied);
    } catch {
      setMsg(att.path);
    }
  }

  return (
    <li className="intake__file">
      <div className="intake__file-row">
        <span className="intake__file-icon" aria-hidden>📄</span>
        <span className="intake__file-name" title={att.path}>{att.filename}</span>
        <span className="intake__meta">{formatSize(att.size)}</span>
        {msg && <span className="intake__meta">{msg}</span>}
        <span className="intake__file-actions">
          {isText && (
            <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void togglePreview()}>
              {open ? t.intake_file_hide : t.intake_file_preview}
            </button>
          )}
          <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void copyPath()}>
            {t.intake_file_copy_path}
          </button>
        </span>
      </div>
      {open && preview !== null && <pre className="intake__file-preview">{preview}</pre>}
    </li>
  );
}

function Attachments({ itemId }: { itemId: string }) {
  const { t } = useLocale();
  const [atts, setAtts] = useState<IntakeAttachment[]>([]);
  const [urls, setUrls] = useState<Record<string, string>>({});
  const [failed, setFailed] = useState<Record<string, true>>({});
  const [zoomId, setZoomId] = useState<string | null>(null);

  useEffect(() => {
    if (!zoomId) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setZoomId(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [zoomId]);

  useEffect(() => {
    let alive = true;
    intakeAttachments(itemId)
      .then(async (list) => {
        if (!alive) return;
        setAtts(list);
        // 只有圖片需要載入內容做縮圖；其他檔案列出檔名與大小，純文字另外提供預覽。
        for (const a of list.filter((x) => x.mime.startsWith("image/"))) {
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
  const images = atts.filter((a) => a.mime.startsWith("image/"));
  const files = atts.filter((a) => !a.mime.startsWith("image/"));
  return (
    <section>
      <h4 className="intake__block-title">{t.intake_attachments.replace("{n}", String(atts.length))}</h4>
      {images.length > 0 && (
      <div className="intake__images">
        {images.map((a) =>
          urls[a.id] ? (
            <button key={a.id} type="button" className="intake__thumb-btn" title={a.filename} onClick={() => setZoomId(a.id)}>
              <img src={urls[a.id]} alt={a.filename} className="intake__thumb" />
            </button>
          ) : (
            <span key={a.id} className="intake__thumb intake__thumb--empty">
              {failed[a.id] ? t.intake_attachment_failed : a.filename}
            </span>
          ),
        )}
      </div>
      )}
      {files.length > 0 && (
        <ul className="intake__files">
          {files.map((a) => (
            <FileRow key={a.id} att={a} />
          ))}
        </ul>
      )}
      {zoomId && urls[zoomId] && (
        <div className="intake__lightbox" role="dialog" aria-label={atts.find((a) => a.id === zoomId)?.filename} onClick={() => setZoomId(null)}>
          <button type="button" className="intake__lightbox-close" aria-label={t.intake_image_close} onClick={() => setZoomId(null)}>✕</button>
          <img src={urls[zoomId]} alt={atts.find((a) => a.id === zoomId)?.filename ?? ""} className="intake__lightbox-img" onClick={(e) => e.stopPropagation()} />
        </div>
      )}
    </section>
  );
}

function formatTime(sec: number): string {
  return new Date(sec * 1000).toLocaleString(undefined, { year: "numeric", month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

/** 二次確認的刪除鈕（Tauri 視窗裡 window.confirm 不可靠）。處理中的不能刪。 */
function DeleteButton({ item, run, variant }: { item: IntakeItem; run: (fn: () => Promise<unknown>) => Promise<void>; variant: "row" | "pane" }) {
  const { t } = useLocale();
  const [armed, setArmed] = useState(false);
  if (item.status === "running") return null;
  return (
    <button
      type="button"
      className={`intake__del intake__del--${variant} ${armed ? "intake__del--armed" : ""}`}
      title={t.intake_action_delete}
      aria-label={t.intake_action_delete}
      onClick={(e) => {
        e.stopPropagation();
        if (armed) void run(() => intakeDelete(item.id));
        else setArmed(true);
      }}
      onBlur={() => setArmed(false)}
    >
      {armed ? t.intake_action_delete_confirm : variant === "row" ? "✕" : t.intake_action_delete}
    </button>
  );
}

function FlowBar({ item }: { item: IntakeItem }) {
  const { t } = useLocale();
  const steps = flowSteps(item);
  return (
    <ol className="intake__flow" aria-label={t.intake_flow_label}>
      {steps.map((s, i) => (
        <li
          key={s.key}
          className={`intake__flow-step intake__flow-step--${s.state}`}
          aria-current={s.state === "current" ? "step" : undefined}
          tabIndex={0}
        >
          <span className="intake__flow-tip" role="tooltip">
            <strong>{t[`intake_flow_${s.key}` as const]}</strong>
            <span>{t[`intake_flow_desc_${s.key}` as const]}</span>
            <em className={`intake__flow-tip-state intake__flow-tip-state--${s.state}`}>{t[`intake_flow_state_${s.state}` as const]}</em>
          </span>
          <span className="intake__flow-dot">{s.state === "done" ? "✓" : s.state === "failed" ? "!" : s.state === "skipped" ? "–" : i + 1}</span>
          <span className="intake__flow-label">{t[`intake_flow_${s.key}` as const]}</span>
        </li>
      ))}
    </ol>
  );
}

function Detail({ item, run, projects }: { item: IntakeItem; run: (fn: () => Promise<unknown>) => Promise<void>; projects: IntakeProject[] }) {
  const { t } = useLocale();
  const [draft, setDraft] = useState(item.reply_draft ?? "");
  const [preview, setPreview] = useState(false);
  const [assignTo, setAssignTo] = useState("");
  const hasReply = (item.status === "draft" || item.status === "done") && item.reply_draft !== null;
  // 沒有任何可按的按鈕（例如已完成的題目）就不顯示底部操作列，回覆卡片可以一路貼到面板底部。
  const hasActions = item.status === "pending" || item.status === "queued" || (item.status === "draft" && hasReply) || item.project_id === "";
  return (
    <div className="intake__pane">
      <div className="intake__pane-top">
        <DeleteButton item={item} run={run} variant="pane" />
        <FlowBar item={item} />
      </div>
      <header className="intake__head">
        <h3>{item.subject || t.intake_no_subject}</h3>
        <div className="intake__chips">
          <span className="intake__asker">
            <span className="intake__asker-label">{t.intake_asker}</span>
            {item.sender_name && <strong>{item.sender_name}</strong>}
            {item.sender_name ? <span className="intake__meta">{item.sender}</span> : <strong>{item.sender}</strong>}
          </span>
          <span className={`intake__level intake__level--${item.sender_level}`}>
            {item.sender_level === "internal" ? t.intake_level_internal : t.intake_level_external}
          </span>
          <span className="intake__chip">{item.source}</span>
          {item.kind && <span className="intake__chip">{item.kind === "code_change" ? t.intake_kind_code_change : t.intake_kind_question}</span>}
          <span className="intake__meta">{new Date(item.created_at * 1000).toLocaleString()}</span>
        </div>
      </header>
      {item.error && <div className="intake__banner">{item.error}</div>}

      <section className="intake__block">
        <h4 className="intake__block-title">{t.intake_block_question}</h4>
        <pre className="intake__text">{item.body}</pre>
        <Attachments itemId={item.id} />
      </section>

      {item.verification && (
        <section className="intake__block">
          <h4 className={`intake__block-title ${item.verified ? "intake__ok" : "intake__error"}`}>
            {item.verified ? t.intake_verify_passed : t.intake_verify_failed}
          </h4>
          <pre className="intake__text">{item.verification}</pre>
        </section>
      )}

      {hasReply && (
        <section className="intake__block intake__block--grow">
          <div className="intake__block-head">
            <h4 className="intake__block-title">{t.intake_reply}</h4>
            {item.status === "draft" && (
              <div className="intake__seg" role="group">
                <button type="button" className={!preview ? "is-on" : ""} onClick={() => setPreview(false)}>{t.intake_edit}</button>
                <button type="button" className={preview ? "is-on" : ""} onClick={() => setPreview(true)}>{t.intake_preview}</button>
              </div>
            )}
          </div>
          {item.status === "done" || preview ? (
            <div className="intake__md"><MarkdownText text={item.status === "done" ? item.reply_draft ?? "" : draft} /></div>
          ) : (
            <textarea className="intake__draft" value={draft} onChange={(e) => setDraft(e.target.value)} rows={10} />
          )}
        </section>
      )}

      {hasActions && (
      <footer className="intake__footer">
        {item.project_id === "" && (
          <>
            <select aria-label={t.intake_assign_pick} value={assignTo} onChange={(e) => setAssignTo(e.target.value)}>
              <option value="">{t.intake_assign_pick}</option>
              {projects.map((p) => (
                <option key={p.id} value={p.id}>{p.name}</option>
              ))}
            </select>
            <button className="aiterm-btn aiterm-btn--secondary" disabled={!assignTo} onClick={() => void run(() => intakeAssign(item.id, assignTo))}>
              {t.intake_assign}
            </button>
          </>
        )}
        {item.status === "pending" && (
          <>
            <button className="aiterm-btn aiterm-btn--primary" onClick={() => void run(() => intakeApprove(item.id))}>
              {t.intake_action_approve}
            </button>
            <button className="aiterm-btn aiterm-btn--secondary" onClick={() => void run(() => intakeReject(item.id))}>
              {t.intake_action_reject}
            </button>
          </>
        )}
        {item.status === "queued" && (
          <button className="aiterm-btn aiterm-btn--secondary" onClick={() => void run(() => intakeReject(item.id))}>
            {t.intake_action_reject}
          </button>
        )}
        {item.status === "draft" && hasReply && (
          <>
            <button
              className="aiterm-btn aiterm-btn--primary"
              onClick={() =>
                void run(async () => {
                  await intakeUpdateDraft(item.id, draft);
                  await intakeSendReply(item.id);
                })
              }
            >
              {t.intake_action_send}
            </button>
            <button className="aiterm-btn aiterm-btn--secondary" onClick={() => void run(() => intakeUpdateDraft(item.id, draft))}>
              {t.intake_action_save_draft}
            </button>
          </>
        )}
      </footer>
      )}
    </div>
  );
}

function SettingsPanel({ projectId, projectName, legacySettings, onError }: { projectId: string; projectName: string; legacySettings: boolean; onError: (m: string | null) => void }) {
  const { t } = useLocale();
  const [cfg, setCfg] = useState<IntakeConfig | null>(null);
  const [hasOwn, setHasOwn] = useState(true);
  const [chanTab, setChanTab] = useState<"mail" | "telegram" | "web">("mail");
  const [tgDefault, setTgDefault] = useState(false);
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  const [senders, setSenders] = useState<IntakeSender[]>([]);
  const [dropped, setDropped] = useState<[string, number][]>([]);
  const [newAddr, setNewAddr] = useState("");
  const [newLevel, setNewLevel] = useState<Level>("internal");
  const [saved, setSaved] = useState(false);

  const loadSenders = useCallback(() => {
    intakeSenders(projectId).then(setSenders).catch((e) => onError(errText(e)));
    intakeDropped(projectId).then(setDropped).catch(() => {});
  }, [onError, projectId]);

  useEffect(() => {
    intakeGetConfig(projectId)
      .then((v) => {
        setCfg(v.config);
        setHasOwn(v.saved);
        setTgDefault(v.telegram_default);
      })
      .catch((e) => onError(errText(e)));
    mailListAccounts().then(setAccounts).catch(() => {});
    loadSenders();
  }, [loadSenders, onError, projectId]);

  if (!cfg) return null;
  const set = <K extends keyof IntakeConfig>(k: K, v: IntakeConfig[K]) => {
    setSaved(false);
    setCfg({ ...cfg, [k]: v });
  };

  async function save() {
    try {
      await intakeSetConfig(projectId, cfg!, tgDefault);
      setSaved(true);
      setHasOwn(true);
      onError(null);
    } catch (e) {
      onError(errText(e));
    }
  }

  async function addSender() {
    try {
      await intakeUpsertSender(projectId, { address: newAddr.trim(), level: newLevel, skip_auth: false });
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
      <div className="intake__editing">{t.intake_editing_project.replace("{name}", projectName)}</div>
      {!hasOwn && legacySettings && (
        <div className="intake__banner intake__adopt">
          <span>{t.intake_copy_legacy_hint}</span>
          <button
            className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
            onClick={() =>
              void intakeCopyLegacyConfig(projectId)
                .then(() => intakeGetConfig(projectId))
                .then((v) => {
                  setCfg(v.config);
                  setHasOwn(v.saved);
                  loadSenders();
                })
                .catch((e) => onError(errText(e)))
            }
          >
            {t.intake_copy_legacy}
          </button>
        </div>
      )}
      <label className="intake__enable">
        <input type="checkbox" aria-label={t.intake_set_enabled} checked={cfg.enabled} onChange={(e) => set("enabled", e.target.checked)} />
        <span className="intake__enable-text">
          <strong>{t.intake_set_enabled}</strong>
          <span className="intake__meta">{t.intake_enable_desc}</span>
        </span>
      </label>
      <section className="intake__card">
        <h3 className="intake__card-title">{t.intake_sec_channels}</h3>
        <div className="intake__tabs" role="tablist">
          {([
            ["mail", t.intake_sec_mailbox, cfg.account_id ? t.intake_tab_set : t.intake_tab_unset, !!cfg.account_id],
            ["telegram", t.intake_sec_telegram, cfg.telegram_enabled ? t.intake_tab_on : t.intake_tab_off, cfg.telegram_enabled],
            ["web", t.intake_sec_web, cfg.web_mode !== "off" ? t.intake_tab_on : t.intake_tab_off, cfg.web_mode !== "off"],
          ] as const).map(([key, label, status, on]) => (
            <button
              key={key}
              type="button"
              role="tab"
              aria-selected={chanTab === key}
              className={`intake__tab ${chanTab === key ? "intake__tab--active" : ""}`}
              onClick={() => setChanTab(key)}
            >
              {label}
              <span className={`intake__tab-chip ${on ? "intake__tab-chip--on" : ""}`}>{status}</span>
            </button>
          ))}
        </div>
        <div className="intake__tabpanel" role="tabpanel">
        {chanTab === "mail" && (
          <>
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
          </>
        )}
        {chanTab === "telegram" && (
          <>
      <label className="intake__field">
        <input type="checkbox" checked={cfg.telegram_enabled} onChange={(e) => set("telegram_enabled", e.target.checked)} />
        {t.intake_set_telegram}
      </label>
      <p className="intake__meta intake__hint">{t.intake_set_telegram_hint}</p>
      <label className="intake__field">
        <input type="checkbox" checked={tgDefault} onChange={(e) => { setSaved(false); setTgDefault(e.target.checked); }} />
        {t.intake_set_tg_default}
      </label>
          </>
        )}
        {chanTab === "web" && (
          <>
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
            aria-label={`${t.intake_set_web_port} ${t.intake_web_port_shared}`}
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
          </>
        )}
        </div>
      </section>
      <div className="intake__cols">
      <div className="intake__col">
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
      </div>
      <div className="intake__col">
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
      </div>
      </div>
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
                  onChange={(e) => void intakeUpsertSender(projectId, { ...s, skip_auth: e.target.checked }).then(loadSenders)}
                />
                {t.intake_skip_auth}
              </label>
            )}
            <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void intakeDeleteSender(projectId, s.address).then(loadSenders)}>
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
