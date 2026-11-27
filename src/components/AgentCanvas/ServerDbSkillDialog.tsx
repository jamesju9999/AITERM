import { confirm } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { useIntakeRemote } from "../../hooks/useIntakeRemote";
import type { DbSkill, DbSkillMode } from "../../ipc/agents";
import { dbListConnections, type DbConnectionInfo } from "../../ipc/db";
import { deleteServerDbConn, listServerDbConns, shareDbConnection, testServerDbConn, type ServerDbConn } from "../../ipc/serverDb";
import { RoleIcon } from "./RoleIcon";

/** server 支援的資料庫類型（SQL Server、DB2 只在桌面版）。 */
const SUPPORTED = ["postgresql", "mysql", "sqlite"];

const errText = (e: unknown) => (typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));

/**
 * 共用專案的 Agent 資料庫技能：從 server 上分享的連線勾選，並選權限。
 * 管理員可以把這台電腦上設定好的連線「分享到 server」（帳密由 server 加密保存）、測試與刪除；其他成員只能看。
 */
export function ServerDbSkillDialog({
  agentName,
  current,
  onSave,
  onClose,
}: {
  agentName: string;
  current: DbSkill | null;
  onSave: (skill: DbSkill | null) => Promise<void>;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const isAdmin = useIntakeRemote().role === "admin";
  const [conns, setConns] = useState<ServerDbConn[] | null>(null);
  const [local, setLocal] = useState<DbConnectionInfo[]>([]);
  const [picked, setPicked] = useState<string[]>(current?.connections ?? []);
  const [mode, setMode] = useState<DbSkillMode>(current?.mode ?? "read");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const reload = async () => setConns(await listServerDbConns());

  useEffect(() => {
    let alive = true;
    listServerDbConns()
      .then((c) => alive && setConns(c))
      .catch((e) => {
        if (!alive) return;
        setConns([]);
        setError(errText(e));
      });
    if (isAdmin) {
      dbListConnections()
        .then((c) => alive && setLocal(c))
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

  const share = (c: DbConnectionInfo) =>
    run(async () => {
      const shared = await shareDbConnection(c.id);
      await reload();
      setPicked((p) => (p.includes(shared.id) ? p : [...p, shared.id]));
      setNotice(t.ac_sdb_shared(shared.name));
    });

  const test = (c: ServerDbConn) =>
    run(async () => {
      await testServerDbConn(c.id);
      setNotice(t.ac_sdb_test_ok(c.name));
    });

  const remove = (c: ServerDbConn) =>
    run(async () => {
      if (!(await confirm(t.ac_sdb_delete_confirm(c.name), { kind: "warning" }))) return;
      await deleteServerDbConn(c.id);
      await reload();
      setPicked((p) => p.filter((id) => id !== c.id));
    });

  const save = (skill: DbSkill | null) =>
    run(async () => {
      await onSave(skill);
      onClose();
    });

  const modes: { value: DbSkillMode; label: string; hint: string }[] = [
    { value: "read", label: t.ac_db_mode_read, hint: t.ac_db_mode_read_hint },
    { value: "approve", label: t.ac_db_mode_approve, hint: t.ac_sdb_mode_approve_hint },
    { value: "write", label: t.ac_db_mode_write, hint: t.ac_db_mode_write_hint },
  ];
  const unshared = local.filter((l) => !(conns ?? []).some((c) => c.name === l.name));

  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-db-dialog" role="dialog" aria-label={t.ac_db_title(agentName)} data-testid="server-db-skill-dialog">
        <h3 className="task-dialog-title ac-inline-icon">
          <span className="ac-agent-ico ac-skill-ico-db" aria-hidden>
            <RoleIcon name="database" />
          </span>
          {t.ac_db_title(agentName)}
        </h3>
        <p className="ac-dialog-desc">{t.ac_sdb_desc}</p>

        <div className="ac-db-section">{t.ac_db_connections}</div>
        {conns === null && !error && <p className="task-field-hint">{t.ac_db_loading}</p>}
        {conns?.length === 0 && !error && <p className="task-field-hint">{isAdmin ? t.ac_sdb_none_admin : t.ac_sdb_none_member}</p>}
        {!!conns?.length && (
          <div className="ac-db-conns" data-testid="server-db-conns">
            {conns.map((c) => (
              <div key={c.id} className="ac-db-option ac-ss-row">
                <label className="ac-ss-label">
                  <input
                    type="checkbox"
                    aria-label={c.name}
                    checked={picked.includes(c.id)}
                    disabled={!isAdmin}
                    onChange={(e) => setPicked((p) => (e.target.checked ? [...p, c.id] : p.filter((x) => x !== c.id)))}
                  />
                  <span>
                    {c.name} <span className="ac-db-meta">{[c.db_type, c.database].filter(Boolean).map((s) => `· ${s}`).join(" ")}</span>
                  </span>
                </label>
                {isAdmin && (
                  <span className="ac-ss-actions">
                    <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={() => void test(c)} aria-label={`${t.ac_sdb_test} ${c.name}`}>
                      {t.ac_sdb_test}
                    </button>
                    <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={() => void remove(c)} aria-label={`${t.ac_ss_delete} ${c.name}`}>
                      {t.ac_ss_delete}
                    </button>
                  </span>
                )}
              </div>
            ))}
          </div>
        )}

        {isAdmin && unshared.length > 0 && (
          <div data-testid="server-db-share">
            <div className="ac-db-section">{t.ac_sdb_share_title}</div>
            {unshared.map((l) => {
              const ok = SUPPORTED.includes(l.db_type);
              return (
                <div key={l.id} className="ac-db-option ac-ss-row">
                  <span>
                    {l.name} <span className="ac-db-meta">· {l.db_type}</span>
                  </span>
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm"
                    disabled={busy || !ok}
                    title={ok ? undefined : t.ac_sdb_unsupported}
                    onClick={() => void share(l)}
                    aria-label={`${t.ac_sdb_share} ${l.name}`}
                  >
                    {ok ? t.ac_sdb_share : t.ac_sdb_unsupported_short}
                  </button>
                </div>
              );
            })}
            <p className="task-field-hint">{t.ac_sdb_share_hint}</p>
          </div>
        )}

        <div className="ac-db-section">{t.ac_db_permission}</div>
        {modes.map((m) => (
          <label key={m.value} className="ac-db-option">
            <input type="radio" name="ac-sdb-mode" checked={mode === m.value} disabled={!isAdmin} onChange={() => setMode(m.value)} aria-label={m.label} />
            <span>
              {m.label} <span className="ac-db-meta">{m.hint}</span>
            </span>
          </label>
        ))}
        {!isAdmin && <p className="task-field-hint">{t.ac_sdb_readonly}</p>}

        {notice && !error && <p className="ac-good-text">{notice}</p>}
        {error && (
          <p className="ac-bad-text" role="alert">
            {error}
          </p>
        )}
        <div className="task-dialog-actions">
          {isAdmin && current && (
            <button className="aiterm-btn aiterm-btn--ghost ac-db-remove" disabled={busy} onClick={() => void save(null)}>
              {t.ac_db_remove}
            </button>
          )}
          <span className="ac-spacer" />
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {isAdmin ? t.cancel : t.ac_done_close}
          </button>
          {isAdmin && (
            <button className="aiterm-btn aiterm-btn--primary" disabled={busy || picked.length === 0} onClick={() => void save({ connections: picked, mode })}>
              {t.ac_db_save}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
