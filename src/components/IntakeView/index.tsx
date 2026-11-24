import { useCallback, useEffect, useState, type ReactNode } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  intakeAdoptLegacy,
  intakeCheckFolders,
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
  intakeLineClearCredentials,
  intakeLineForgetUnknown,
  intakeLineUnknown,
  intakeLineSetCredentials,
  intakeLineStatus,
  intakeList,
  intakeProjects,
  intakeRefresh,
  intakeReject,
  intakeSendReply,
  intakeSenders,
  intakeSetSenderLabel,
  intakeSetConfig,
  intakeUpdateDraft,
  intakeUpsertSender,
  type IntakeAttachment,
  type IntakeChannelHandling,
  type IntakeRefFolders,
  type IntakeFolderCheck,
  type IntakeConfig,
  type IntakeLineStatus,
  type IntakeLineUnknown,
  type IntakeItem,
  type IntakeProject,
  type IntakeSender,
  type IntakeStatus,
  type Level,
} from "../../ipc/intake";
import { MailIcon } from "../Icons";
import type { IntakeRemoteStatus } from "../../ipc/intake";
import { MarkdownText } from "../../lib/markdown";
import { useNavigate } from "react-router-dom";
import { mailListAccounts, type MailAccount } from "../../ipc/mail";
import { systemsList, type SystemConnection } from "../../ipc/systems";
import { useIntakeChanged } from "../../hooks/useIntakeChanged";
import { useIntakeRemote } from "../../hooks/useIntakeRemote";
import { ConnectionCard } from "./ConnectionCard";
import { useLocale } from "../../contexts/LocaleContext";
import { displayAddress, validAddress } from "../../lib/intakeAddress";
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

const STATUSES: IntakeStatus[] = ["pending", "queued", "running", "on_board", "draft", "done", "rejected"];

/** 題目來源（`item.source`）在畫面上的管道名稱。 */
function channelName(source: string, t: ReturnType<typeof useLocale>["t"]): string {
  switch (source) {
    case "email": return t.intake_sec_mailbox;
    case "telegram": return t.intake_sec_telegram;
    case "line": return t.intake_sec_line;
    case "web": return t.intake_sec_web;
    default: return source;
  }
}

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
  const remote = useIntakeRemote();
  const [showConn, setShowConn] = useState(false);

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
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-pressed={showSettings} onClick={() => { setShowConn(false); setShowSettings((v) => !v); }}>
            {showSettings ? t.intake_settings_back : t.intake_settings}
          </button>
        )}
        <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-pressed={showConn} onClick={() => setShowConn((v) => !v)}>
          {t.intake_conn_button}
        </button>
      </div>
      {remote.connected && (
        <div className={`intake__remote ${remote.online && !remote.error ? "" : "intake__remote--off"}`} role="status">
          {t.intake_conn_banner}：{remote.addr}（{remote.name}）{remote.online ? "" : ` — ${t.intake_conn_offline}`}
        </div>
      )}
      {error && <div className="intake__error" role="alert">{error}</div>}

      {showConn ? (
        <ConnectionCard status={remote} />
      ) : noProject ? (
        <p className="intake__empty">{t.intake_no_projects}</p>
      ) : showSettings && !inUnassigned ? (
        <SettingsPanel key={projectId} projectId={projectId} projectName={current?.name ?? ""} legacySettings={legacySettings} remote={remote} onError={setError} />
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
                <span className="intake__refresh-icon" aria-hidden="true">⟳</span>
              </button>
              <select
                aria-label={t.intake_filter_label}
                value={filter}
                onChange={(e) => setFilter(e.target.value as IntakeStatus | "all")}
              >
                <option value="all">{t.intake_filter_all}</option>
                {STATUSES.map((s) => (
                  <option key={s} value={s}>
                    {s === "on_board" ? t.intake_filter_on_card : t[`intake_status_${s}` as const]}
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
                      <span className="intake__rowbadges">
                        <span className={`intake__badge intake__badge--${i.status}`}>{i.status === "on_board" && i.route_kind?.startsWith("canvas") ? t.intake_status_on_canvas : t[`intake_status_${i.status}` as const]}</span>
                        <span className="intake__badge intake__badge--channel" title={t.intake_channel}>{channelName(i.source, t)}</span>
                      </span>
                      <time className="intake__time" dateTime={new Date(i.created_at * 1000).toISOString()}>{formatTime(i.created_at)}</time>
                    </span>
                    <span className="intake__subject">{i.subject || t.intake_no_subject}</span>
                    <span className="intake__sender">{t.intake_asker}：{i.sender_name ? `${i.sender_name} <${displayAddress(i.sender)}>` : displayAddress(i.sender)} · {i.sender_level === "internal" ? t.intake_level_internal : t.intake_level_external}</span>
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

/** 原信的其他收件人與副本：回信會一併寄給他們（自己與提問者除外，實際名單在寄出時才算）。 */
function Recipients({ item }: { item: IntakeItem }) {
  const { t } = useLocale();
  const lines = (v: string | null) => (v ?? "").split("\n").filter(Boolean);
  const others = lines(item.to_addrs).filter((a) => a.toLowerCase() !== item.sender.toLowerCase());
  const cc = lines(item.cc_addrs);
  if (item.source !== "email" || (others.length === 0 && cc.length === 0)) return null;
  return (
    <p className="intake__meta intake__recipients">
      {t.intake_reply_all}：{others.join("、")}
      {cc.length > 0 && `${others.length > 0 ? "；" : ""}${t.intake_reply_cc}：${cc.join("、")}`}
    </p>
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
          <span className="intake__chip" title={t.intake_channel}>{channelName(item.source, t)}</span>
          {item.kind && <span className="intake__chip">{item.kind === "code_change" ? t.intake_kind_code_change : t.intake_kind_question}</span>}
          <span className="intake__meta">{new Date(item.created_at * 1000).toLocaleString()}</span>
        </div>
      </header>
      {item.error && <div className="intake__banner">{item.error}</div>}
      {item.board_task_id && <div className="intake__meta">{item.route_kind?.startsWith("canvas") ? t.intake_on_canvas_note : t.intake_on_board_note}（{item.board_task_id.slice(0, 8)}）</div>}
      <Recipients item={item} />

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

function SettingsPanel({ projectId, projectName, legacySettings, remote, onError }: { projectId: string; projectName: string; legacySettings: boolean; remote: IntakeRemoteStatus; onError: (m: string | null) => void }) {
  const { t } = useLocale();
  const navigate = useNavigate();
  // 連到 server 時：設定是 server 的，只有管理員能改；資料夾是 server 上的路徑，不能用本機的選擇器。
  const readOnly = remote.connected && remote.role !== "admin";
  const [cfg, setCfg] = useState<IntakeConfig | null>(null);
  const [hasOwn, setHasOwn] = useState(true);
  const [chanTab, setChanTab] = useState<"mail" | "telegram" | "line" | "web">("mail");
  const [tgDefault, setTgDefault] = useState(false);
  const [lineStatus, setLineStatus] = useState<IntakeLineStatus | null>(null);
  const [lineUnknown, setLineUnknown] = useState<IntakeLineUnknown[]>([]);
  const [refChecks, setRefChecks] = useState<Record<string, IntakeFolderCheck>>({});
  const [newRefs, setNewRefs] = useState<Record<string, string>>({});
  const [unknownLabels, setUnknownLabels] = useState<Record<string, string>>({});
  const [editingAddr, setEditingAddr] = useState<string | null>(null);
  const [editLabel, setEditLabel] = useState("");
  const [lineSecret, setLineSecret] = useState("");
  const [lineToken, setLineToken] = useState("");
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  const [systems, setSystems] = useState<SystemConnection[]>([]);
  const [senders, setSenders] = useState<IntakeSender[]>([]);
  const [dropped, setDropped] = useState<[string, number][]>([]);
  const [newAddr, setNewAddr] = useState("");
  const [newLevel, setNewLevel] = useState<Level>("internal");
  const [saved, setSaved] = useState(false);
  const [dirty, setDirty] = useState(false);

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
    intakeLineStatus().then(setLineStatus).catch(() => {});
    intakeLineUnknown().then(setLineUnknown).catch(() => {});
    mailListAccounts().then(setAccounts).catch(() => {});
    systemsList().then(setSystems).catch(() => {});
    loadSenders();
  }, [loadSenders, onError, projectId]);

  const refPaths = cfg ? Array.from(new Set([...cfg.reference_folders.shared, ...cfg.reference_folders.internal])) : [];
  const refKey = JSON.stringify(refPaths);
  useEffect(() => {
    const paths = JSON.parse(refKey) as string[];
    if (paths.length === 0) return;
    let live = true;
    intakeCheckFolders(projectId, paths)
      .then((r) => live && setRefChecks(Object.fromEntries(r.map((c) => [c.path, c]))))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [projectId, refKey]);

  if (!cfg) return null;
  const set = <K extends keyof IntakeConfig>(k: K, v: IntakeConfig[K]) => {
    setSaved(false);
    setDirty(true);
    setCfg({ ...cfg, [k]: v });
  };

  async function save() {
    try {
      await intakeSetConfig(projectId, cfg!, tgDefault);
      intakeLineStatus().then(setLineStatus).catch(() => {});
      setSaved(true);
      setDirty(false);
      setHasOwn(true);
      onError(null);
    } catch (e) {
      onError(errText(e));
    }
  }

  async function saveLineCredentials() {
    try {
      await intakeLineSetCredentials(lineSecret, lineToken);
      setLineSecret("");
      setLineToken("");
      setLineStatus(await intakeLineStatus());
      onError(null);
    } catch (e) {
      onError(errText(e));
    }
  }

  async function clearLineCredentials() {
    try {
      await intakeLineClearCredentials();
      setLineStatus(await intakeLineStatus());
      onError(null);
    } catch (e) {
      onError(errText(e));
    }
  }

  async function addLineUnknown(u: IntakeLineUnknown, level: Level) {
    try {
      const label = (unknownLabels[u.id] ?? u.name).trim();
      await intakeUpsertSender(projectId, { address: `line:${u.id}`, level, skip_auth: false, label });
      await intakeLineForgetUnknown(u.id);
      setLineUnknown(await intakeLineUnknown());
      onError(null);
      loadSenders();
    } catch (e) {
      onError(errText(e));
    }
  }

  async function saveLabel(address: string) {
    try {
      await intakeSetSenderLabel(projectId, address, editLabel);
      setEditingAddr(null);
      onError(null);
      loadSenders();
    } catch (e) {
      onError(errText(e));
    }
  }

  async function ignoreLineUnknown(u: IntakeLineUnknown) {
    try {
      await intakeLineForgetUnknown(u.id);
      setLineUnknown(await intakeLineUnknown());
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

  // 很長的說明收進「說明」裡，要看再展開，避免一整頁都是長句。
  const hint = (text: string) => (
    <details className="intake__more">
      <summary>{t.intake_hint_more}</summary>
      <p className="intake__meta intake__hint">{text}</p>
    </details>
  );

  const handlingField = (ch: keyof IntakeChannelHandling, label: string) => (
    <>
      <label className="intake__field">
        {t.intake_handling}
        <select
          aria-label={`${label} ${t.intake_handling}`}
          value={cfg.channel_handling[ch]}
          onChange={(e) => set("channel_handling", { ...cfg.channel_handling, [ch]: e.target.value as IntakeChannelHandling[typeof ch] })}
        >
          <option value="diagnose">{t.intake_handling_diagnose}</option>
          <option value="edit">{t.intake_handling_edit}</option>
        </select>
      </label>
      {hint(cfg.channel_handling[ch] === "diagnose" ? t.intake_handling_diagnose_hint : t.intake_handling_edit_hint)}
    </>
  );

  const addRef = (ch: keyof IntakeRefFolders, raw: string) => {
    const p = raw.trim();
    if (!p || cfg.reference_folders[ch].includes(p)) return;
    set("reference_folders", { ...cfg.reference_folders, [ch]: [...cfg.reference_folders[ch], p] });
    setNewRefs((m) => ({ ...m, [ch]: "" }));
  };
  const removeRef = (ch: keyof IntakeRefFolders, p: string) =>
    set("reference_folders", { ...cfg.reference_folders, [ch]: cfg.reference_folders[ch].filter((x) => x !== p) });
  const pickRef = async (ch: keyof IntakeRefFolders) => {
    try {
      const picked = await openDialog({ directory: true, multiple: false });
      if (typeof picked === "string" && picked) addRef(ch, picked);
    } catch (e) {
      onError(errText(e));
    }
  };
  const refStatus = (c: IntakeFolderCheck | undefined): { ok: boolean; text: string } => {
    if (!c) return { ok: true, text: t.intake_refs_checking };
    if (!c.exists) return { ok: false, text: t.intake_refs_missing };
    if (!c.is_dir) return { ok: false, text: t.intake_refs_not_dir };
    const parts = [c.readable > 0 ? t.intake_refs_ok(c.readable) : t.intake_refs_empty];
    if (c.office > 0) parts.push(t.intake_refs_office(c.office));
    if (c.truncated) parts.push(t.intake_refs_truncated);
    return { ok: c.readable > 0 && c.office === 0, text: parts.join("；") };
  };
  const refList = (ch: keyof IntakeRefFolders, title: string, label: string, note: string) => (
    <div className="intake__refs">
      <h4 className="intake__group-title">{title}</h4>
      <p className="intake__meta intake__hint">{note}</p>
      <ul className="intake__refs-list" aria-label={`${label} ${t.intake_refs}`}>
        {cfg.reference_folders[ch].map((p) => {
          const st = refStatus(refChecks[p]);
          return (
            <li key={p} className="intake__refs-row">
              <span className="intake__refs-main">
                <code>{p}</code>
                <span className={st.ok ? "intake__refs-ok" : "intake__refs-warn"}>
                  <span aria-hidden="true">{st.ok ? "✓" : "⚠"}</span> {st.text}
                </span>
              </span>
              <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-label={`${t.intake_remove} ${p}`} onClick={() => removeRef(ch, p)}>
                {t.intake_remove}
              </button>
            </li>
          );
        })}
      </ul>
      <div className="intake__field intake__field--inline">
        <input
          aria-label={`${label} ${t.intake_refs_path}`}
          value={newRefs[ch] ?? ""}
          placeholder={t.intake_refs_ph}
          onChange={(e) => setNewRefs((m) => ({ ...m, [ch]: e.target.value }))}
          onKeyDown={(e) => {
            if (e.key === "Enter") addRef(ch, newRefs[ch] ?? "");
          }}
        />
        <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-label={`${label} ${t.intake_refs_add}`} disabled={!(newRefs[ch] ?? "").trim()} onClick={() => addRef(ch, newRefs[ch] ?? "")}>
          {t.intake_refs_add}
        </button>
        {!remote.connected && (
          <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-label={`${label} ${t.intake_refs_pick}`} onClick={() => void pickRef(ch)}>
            {t.intake_refs_pick}
          </button>
        )}
      </div>
      {remote.connected && <p className="intake__meta intake__hint">{t.intake_conn_server_path}</p>}
    </div>
  );

  const group = (title: string, children: ReactNode) => (
    <div className="intake__group">
      <h4 className="intake__group-title">{title}</h4>
      {children}
    </div>
  );

  const needsAlias = cfg.mode === "alias" || cfg.mode === "alias_or_tag";
  const needsTag = cfg.mode === "subject_tag" || cfg.mode === "alias_or_tag";

  return (
    <div className="intake__settings">
      <div className="intake__editing">{t.intake_editing_project.replace("{name}", projectName)}</div>
      {readOnly && <div className="intake__meta intake__readonly" role="status">{t.intake_conn_readonly}</div>}
      {remote.connected && <div className="intake__meta intake__hint">{t.intake_conn_channels_note}</div>}
      <fieldset className="intake__fieldset" disabled={readOnly}>
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
            ["line", t.intake_sec_line, cfg.line_enabled ? t.intake_tab_on : t.intake_tab_off, cfg.line_enabled],
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
            <option key={a.id} value={a.id}>{a.enabled === false ? `${a.email}（${t.intake_account_disabled_tag}）` : a.email}</option>
          ))}
        </select>
      </label>
      {accounts.some((a) => a.id === cfg.account_id && a.enabled === false) && (
        <div className="intake__meta" role="alert" style={{ color: "#f59e0b", fontSize: 12, display: "flex", flexWrap: "wrap", alignItems: "center", gap: 8 }}>
          <span>{t.intake_account_disabled_warn}</span>
          <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => navigate("/settings", { state: { tab: "mail" } })}>
            {t.intake_account_disabled_goto}
          </button>
        </div>
      )}
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
      {group(t.intake_grp_handling, handlingField("email", t.intake_sec_mailbox))}
          </>
        )}
        {chanTab === "telegram" && (
          <>
      <label className="intake__field">
        <input type="checkbox" checked={cfg.telegram_enabled} onChange={(e) => set("telegram_enabled", e.target.checked)} />
        {t.intake_set_telegram}
      </label>
      {hint(t.intake_set_telegram_hint)}
      <label className="intake__field">
        <input type="checkbox" checked={tgDefault} onChange={(e) => { setSaved(false); setDirty(true); setTgDefault(e.target.checked); }} />
        {t.intake_set_tg_default}
      </label>
      {group(t.intake_grp_handling, handlingField("telegram", t.intake_sec_telegram))}
          </>
        )}
        {chanTab === "line" && (
          <>
      <label className="intake__field">
        <input type="checkbox" checked={cfg.line_enabled} onChange={(e) => set("line_enabled", e.target.checked)} />
        {t.intake_set_line}
      </label>
      {hint(t.intake_set_line_hint)}
      {group(t.intake_grp_connect, (
        <>
      <label className="intake__field">
        {t.intake_line_secret}
        <input type="password" autoComplete="off" value={lineSecret} placeholder={lineStatus?.has_secret ? t.intake_line_saved : ""} onChange={(e) => setLineSecret(e.target.value)} />
      </label>
      <label className="intake__field">
        {t.intake_line_token}
        <input type="password" autoComplete="off" value={lineToken} placeholder={lineStatus?.has_token ? t.intake_line_saved : ""} onChange={(e) => setLineToken(e.target.value)} />
      </label>
      <div className="intake__field intake__field--row">
        <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={!lineSecret.trim() && !lineToken.trim()} onClick={saveLineCredentials}>
          {t.intake_line_save_creds}
        </button>
        {(lineStatus?.has_secret || lineStatus?.has_token) && (
          <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={clearLineCredentials}>
            {t.intake_line_clear_creds}
          </button>
        )}
        <span className="intake__meta intake__creds-note">{t.intake_line_creds_note}</span>
      </div>
      <label className="intake__field">
        {t.intake_set_line_port}
        <input
          type="number"
          aria-label={t.intake_set_line_port}
          min={1024}
          max={65535}
          value={cfg.line_port}
          onChange={(e) => set("line_port", Math.min(65535, Math.max(1024, Number(e.target.value) || 8321)))}
        />
      </label>
      <p className="intake__meta">{t.intake_line_webhook}：http://127.0.0.1:{cfg.line_port}/line/webhook</p>
      {cfg.line_enabled && lineStatus && !lineStatus.has_secret && <p className="intake__error">{t.intake_line_need_secret}</p>}
      {/* 設定還沒儲存時 server／本機根本還沒開始聽，這時說「埠號被占用」是誤報；存了還沒在聽才是真的問題。 */}
      {cfg.line_enabled && lineStatus?.has_secret && !lineStatus.listening && dirty && <p className="intake__meta intake__warn">{t.intake_line_unsaved_hint}</p>}
      {cfg.line_enabled && lineStatus?.has_secret && !lineStatus.listening && !dirty && <p className="intake__error">{t.intake_line_not_listening}</p>}
      {cfg.line_enabled && lineStatus?.has_secret && !lineStatus.has_token && <p className="intake__error">{t.intake_line_need_token}</p>}
        </>
      ))}
      {group(t.intake_grp_behavior, (
        <>
      <label className="intake__field">
        <input type="checkbox" checked={cfg.line_auto_open} onChange={(e) => set("line_auto_open", e.target.checked)} />
        {t.intake_line_auto_open}
      </label>
      {hint(t.intake_line_auto_open_hint)}
      <label className="intake__field">
        <input type="checkbox" checked={cfg.line_test_mode} onChange={(e) => set("line_test_mode", e.target.checked)} />
        {t.intake_line_test_mode}
      </label>
      {cfg.line_test_mode && <p className="intake__error">{t.intake_line_test_mode_on}</p>}
      {hint(t.intake_line_test_mode_hint)}
      <label className="intake__field">
        {t.intake_line_retention}
        <input
          type="number"
          aria-label={t.intake_line_retention}
          min={0}
          max={365}
          value={cfg.line_retention_days}
          onChange={(e) => set("line_retention_days", Math.min(365, Math.max(0, Math.floor(Number(e.target.value)) || 0)))}
        />
      </label>
      {hint(t.intake_line_retention_hint)}
      <p className="intake__meta intake__hint">{t.intake_line_unknown_pointer}</p>
        </>
      ))}
      {group(t.intake_grp_handling, handlingField("line", t.intake_sec_line))}
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
      {group(t.intake_grp_handling, handlingField("web", t.intake_sec_web))}
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
        {t.intake_set_route}
        <select value={cfg.route} onChange={(e) => set("route", e.target.value as IntakeConfig["route"])}>
          <option value="terminal">{t.intake_route_terminal}</option>
          <option value="board">{t.intake_route_board}</option>
          <option value="canvas">{t.intake_route_canvas}</option>
          <option value="system">{t.intake_route_system}</option>
        </select>
      </label>
      <p className="intake__meta intake__hint">{t[`intake_route_hint_${cfg.route}` as const]}</p>
      {cfg.route === "system" && (
        <label className="intake__field">
          {t.intake_set_system}
          <select value={cfg.system_id} onChange={(e) => set("system_id", e.target.value)}>
            <option value="">{t.intake_system_none}</option>
            {systems.map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
        </label>
      )}
      {cfg.route === "system" && !cfg.system_id && <p className="intake__error">{t.intake_system_required}</p>}
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
      <section className="intake__card">
        <h3 className="intake__card-title">{t.intake_refs}</h3>
        {hint(t.intake_refs_hint)}
        {refList("shared", t.intake_refs_shared, t.intake_refs_shared_label, t.intake_refs_shared_note)}
        {refList("internal", t.intake_refs_internal, t.intake_refs_internal_label, t.intake_refs_internal_note)}
      </section>
      </div>
      </div>
      <section className="intake__card intake__card--grow">
      <h3 className="intake__card-title">{t.intake_senders}</h3>
      <ul className="intake__senders">
        {senders.map((s) => (
          <li key={s.address}>
            <span className="intake__sender-main">
              {editingAddr === s.address ? (
                <input
                  autoFocus
                  aria-label={`${t.intake_sender_label} ${displayAddress(s.address)}`}
                  value={editLabel}
                  maxLength={60}
                  placeholder={t.intake_sender_label_ph}
                  onChange={(e) => setEditLabel(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void saveLabel(s.address);
                    if (e.key === "Escape") setEditingAddr(null);
                  }}
                />
              ) : (
                <>
                  {s.label && <strong>{s.label}</strong>}
                  <span className={s.label ? "intake__sender-addr" : undefined}>
                    {displayAddress(s.address)} · {s.level === "internal" ? t.intake_level_internal : t.intake_level_external}
                  </span>
                </>
              )}
            </span>
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
            <span className="intake__sender-actions">
              {editingAddr === s.address ? (
                <>
                  <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void saveLabel(s.address)}>
                    {t.intake_sender_label_save}
                  </button>
                  <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => setEditingAddr(null)}>
                    {t.intake_sender_label_cancel}
                  </button>
                </>
              ) : (
                <button
                  className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                  aria-label={`${s.label ? t.intake_sender_label_edit : t.intake_sender_label_add} ${displayAddress(s.address)}`}
                  onClick={() => {
                    setEditingAddr(s.address);
                    setEditLabel(s.label ?? "");
                  }}
                >
                  {s.label ? t.intake_sender_label_edit : t.intake_sender_label_add}
                </button>
              )}
              <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-label={`${t.intake_remove} ${displayAddress(s.address)}`} onClick={() => void intakeDeleteSender(projectId, s.address).then(loadSenders)}>
                {t.intake_remove}
              </button>
            </span>
          </li>
        ))}
      </ul>
      <div className="intake__field intake__field--inline">
        <input aria-label={t.intake_sender_address} value={newAddr} placeholder={t.intake_sender_ph} onChange={(e) => setNewAddr(e.target.value)} />
        <select aria-label={t.intake_sender_level} value={newLevel} onChange={(e) => setNewLevel(e.target.value as Level)}>
          <option value="internal">{t.intake_level_internal}</option>
          <option value="external">{t.intake_level_external}</option>
        </select>
        <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={!validAddress(newAddr)} onClick={() => void addSender()}>
          {t.intake_add}
        </button>
      </div>
      {lineUnknown.length > 0 && (
        <div className="intake__unknown" aria-label={t.intake_line_unknown}>
          <h4 className="intake__card-title">{t.intake_line_unknown}</h4>
          <p className="intake__meta intake__hint">{t.intake_line_unknown_hint}</p>
          <ul className="intake__unknown-list">
            {lineUnknown.map((u) => {
              const label = u.name || u.id;
              return (
                <li key={u.id} className="intake__unknown-row">
                  <span className="intake__unknown-kind">{u.kind === "group" ? t.intake_line_kind_group : t.intake_line_kind_user}</span>
                  <span className="intake__unknown-main">
                    <strong>{u.name || t.intake_line_unnamed}</strong>
                    <code>{displayAddress(`line:${u.id}`)}</code>
                  </span>
                  <input
                  className="intake__unknown-label"
                  aria-label={`${t.intake_sender_label} ${label}`}
                  value={unknownLabels[u.id] ?? u.name}
                  maxLength={60}
                  placeholder={t.intake_sender_label_ph}
                  onChange={(e) => setUnknownLabels((m) => ({ ...m, [u.id]: e.target.value }))}
                />
                <span className="intake__unknown-count">{u.count} {t.intake_line_times}</span>
                  <span className="intake__unknown-actions">
                    <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-label={`${t.intake_line_add_internal} ${label}`} onClick={() => void addLineUnknown(u, "internal")}>
                      {t.intake_line_add_internal}
                    </button>
                    <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-label={`${t.intake_line_add_external} ${label}`} onClick={() => void addLineUnknown(u, "external")}>
                      {t.intake_line_add_external}
                    </button>
                    <button className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" aria-label={`${t.intake_line_ignore} ${label}`} onClick={() => void ignoreLineUnknown(u)}>
                      {t.intake_line_ignore}
                    </button>
                  </span>
                </li>
              );
            })}
          </ul>
        </div>
      )}
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
        {dirty && !saved && <span className="intake__dirty" role="status">● {t.intake_unsaved}</span>}
      </div>
      </fieldset>
    </div>
  );
}
