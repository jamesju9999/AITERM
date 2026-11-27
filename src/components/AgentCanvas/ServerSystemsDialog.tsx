import { confirm } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { useIntakeRemote } from "../../hooks/useIntakeRemote";
import { systemsList, type SystemConnection } from "../../ipc/systems";
import { deleteServerSystem, deleteServerSystemSession, listServerSystems, shareSystem, shareSystemSession, type ServerSystem } from "../../ipc/serverSystems";
import { startRemoteLogin } from "../../ipc/serverLive";
import { LiveViewDialog, type LiveTarget } from "./LiveViewDialog";
import { RoleIcon } from "./RoleIcon";

const MAX_PICKED = 12;

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

const when = (secs: number) => new Date(secs * 1000).toLocaleString();

/**
 * 共用專案的 Agent 系統連線：從 server 上分享的網頁系統勾選。管理員可以把本機設定好的系統分享上去、
 * 分享登入 session（先在那個系統的瀏覽器視窗登入）、刪除；其他成員只能看。
 */
export function ServerSystemsDialog({
  agentName,
  current,
  onSave,
  onClose,
}: {
  agentName: string;
  current: string[];
  onSave: (ids: string[]) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const isAdmin = useIntakeRemote().role === "admin";
  const [systems, setSystems] = useState<ServerSystem[] | null>(null);
  const [local, setLocal] = useState<SystemConnection[]>([]);
  const [picked, setPicked] = useState<string[]>(current);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [live, setLive] = useState<LiveTarget | null>(null);

  const reload = async () => setSystems(await listServerSystems());

  useEffect(() => {
    let alive = true;
    listServerSystems()
      .then((s) => alive && setSystems(s))
      .catch((e) => {
        if (!alive) return;
        setSystems([]);
        setError(errText(e));
      });
    if (isAdmin) {
      systemsList()
        .then((l) => alive && setLocal(l.filter((x) => (x.kind ?? "web") === "web")))
        .catch(() => {});
    }
    return () => {
      alive = false;
    };
  }, [isAdmin]);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      await fn();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  };

  const share = (l: SystemConnection) =>
    run(async () => {
      const s = await shareSystem(l.id);
      await reload();
      setPicked((p) => (p.includes(s.id) ? p : [...p, s.id]));
      setNotice(t.ac_ssys_shared(s.name));
    });

  const shareSession = (s: ServerSystem) =>
    run(async () => {
      const l = local.find((x) => x.name === s.name);
      if (!l) throw new Error(t.ac_ssys_no_local(s.name));
      const n = await shareSystemSession(l.id);
      await reload();
      setNotice(t.ac_ssys_session_shared(s.name, n));
    });

  /** 在 server 上遠端登入：開一個瀏覽器、看它的畫面。已經有人在登入就直接去看那個畫面。 */
  const remoteLogin = (s: ServerSystem) =>
    run(async () => {
      try {
        await startRemoteLogin(s.id);
      } catch (e) {
        if (!errText(e).includes("已經有人在遠端登入")) throw e;
      }
      setLive({ kind: "login", systemId: s.id, systemName: s.name });
    });

  const dropSession = (s: ServerSystem) =>
    run(async () => {
      await deleteServerSystemSession(s.id);
      await reload();
    });

  const remove = (s: ServerSystem) =>
    run(async () => {
      if (!(await confirm(t.ac_ssys_delete_confirm(s.name), { kind: "warning" }))) return;
      await deleteServerSystem(s.id);
      await reload();
      setPicked((p) => p.filter((id) => id !== s.id));
    });

  const save = (ids: string[]) =>
    run(async () => {
      await onSave(ids);
      onClose();
    });

  const toggle = (id: string, on: boolean) => {
    if (on && picked.length >= MAX_PICKED) {
      setError(t.ac_ssys_limit(MAX_PICKED));
      return;
    }
    setError(null);
    setPicked((p) => (on ? [...p, id] : p.filter((x) => x !== id)));
  };

  const modeLabel = (m: ServerSystem["mode"]) => ({ read_only: t.sys_mode_title_read_only, approve_each: t.sys_mode_title_approve_each, autonomous: t.sys_mode_title_autonomous })[m];
  const unshared = local.filter((l) => !(systems ?? []).some((s) => s.name === l.name));
  const noSession = (systems ?? []).some((s) => picked.includes(s.id) && !s.session);

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-db-dialog" role="dialog" aria-label={t.ac_sys_title(agentName)} data-testid="server-systems-dialog">
        <h3 className="task-dialog-title ac-inline-icon">
          <span className="ac-agent-ico" aria-hidden>
            <RoleIcon name="globe" />
          </span>
          {t.ac_sys_title(agentName)}
        </h3>
        <p className="ac-dialog-desc">{t.ac_ssys_desc}</p>

        {systems === null && !error && <p className="task-field-hint">{t.ac_db_loading}</p>}
        {systems?.length === 0 && !error && <p className="task-field-hint">{isAdmin ? t.ac_ssys_none_admin : t.ac_ssys_none_member}</p>}
        {!!systems?.length && (
          <div className="ac-db-conns" data-testid="server-systems-list">
            {systems.map((s) => (
              <div key={s.id} className="ac-db-option ac-ss-row">
                <label className="ac-ss-label">
                  <input type="checkbox" aria-label={s.name} checked={picked.includes(s.id)} disabled={!isAdmin || !s.enabled} onChange={(e) => toggle(s.id, e.target.checked)} />
                  <span className="ac-cs-text">
                    <span className="ac-cs-name">
                      {s.name} <span className="ac-db-meta">· {modeLabel(s.mode)}{s.enabled ? "" : ` · ${t.ac_sys_disabled}`}</span>
                    </span>
                    <span className="ac-db-meta">{s.start_url}</span>
                    <span className={s.session ? "ac-db-meta" : "ac-db-meta ac-bad-text"} data-testid={`session-${s.id}`}>
                      {s.session ? t.ac_ssys_session_info(s.session.exported_by, when(s.session.exported_at), s.session.cookie_count) : t.ac_ssys_no_session}
                    </span>
                  </span>
                </label>
                {isAdmin && (
                  <span className="ac-ss-actions">
                    <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={() => void remoteLogin(s)} aria-label={`${t.ac_live_open} ${s.name}`}>
                      {t.ac_live_open}
                    </button>
                    <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={() => void shareSession(s)} aria-label={`${t.ac_ssys_share_session} ${s.name}`}>
                      {t.ac_ssys_share_session}
                    </button>
                    {s.session && (
                      <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={() => void dropSession(s)} aria-label={`${t.ac_ssys_drop_session} ${s.name}`}>
                        {t.ac_ssys_drop_session}
                      </button>
                    )}
                    <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={() => void remove(s)} aria-label={`${t.ac_ss_delete} ${s.name}`}>
                      {t.ac_ss_delete}
                    </button>
                  </span>
                )}
              </div>
            ))}
          </div>
        )}

        {isAdmin && unshared.length > 0 && (
          <div data-testid="server-systems-share">
            <div className="ac-db-section">{t.ac_ssys_share_title}</div>
            <div className="ac-db-conns">
            {unshared.map((l) => {
              const ok = l.mode !== "rules";
              return (
                <div key={l.id} className="ac-db-option ac-ss-row">
                  <span>
                    {l.name} <span className="ac-db-meta">· {l.start_url}</span>
                  </span>
                  <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy || !ok} title={ok ? undefined : t.ac_ssys_rules_unsupported} onClick={() => void share(l)} aria-label={`${t.ac_sdb_share} ${l.name}`}>
                    {ok ? t.ac_sdb_share : t.ac_sdb_unsupported_short}
                  </button>
                </div>
              );
            })}
            </div>
            <p className="task-field-hint">{t.ac_ssys_share_hint}</p>
          </div>
        )}
        {isAdmin && !!systems?.length && <p className="task-field-hint">{t.ac_live_open_hint}</p>}
        {!isAdmin && <p className="task-field-hint">{t.ac_ssys_readonly}</p>}
        {isAdmin && noSession && <p className="task-field-hint ac-bad-text">{t.ac_ssys_picked_no_session}</p>}

        {notice && !error && <p className="ac-good-text">{notice}</p>}
        {error && (
          <p className="ac-bad-text" role="alert">
            {error}
          </p>
        )}
        {live && (
          <LiveViewDialog
            target={live}
            onClose={() => {
              setLive(null);
              void reload();
            }}
            onLoginDone={() => void reload()}
          />
        )}
        <div className="task-dialog-actions">
          {isAdmin && current.length > 0 && (
            <button className="aiterm-btn aiterm-btn--ghost ac-db-remove" disabled={busy} onClick={() => void save([])}>
              {t.ac_sys_remove}
            </button>
          )}
          <span className="ac-spacer" />
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {isAdmin ? t.cancel : t.ac_done_close}
          </button>
          {isAdmin && (
            <button className="aiterm-btn aiterm-btn--primary" disabled={busy || systems === null} onClick={() => void save((systems ?? []).map((s) => s.id).filter((id) => picked.includes(id)))}>
              {t.ac_sys_save}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
