import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { useIntakeRemote } from "../../hooks/useIntakeRemote";
import { getVerify, setVerify, type VerifySettings } from "../../ipc/serverVerify";
import { DenyListDialog } from "./DenyListDialog";

/**
 * 共用看板專案的「由 server 驗收」開關與禁止清單入口。
 * 只在共用專案出現（本機專案 `getVerify` 回 null，什麼都不畫）。只有管理員能開關與編輯禁止清單，
 * 其他人看得到狀態；沒有驗收者時開關停用並說明原因（server 也會擋）。
 */
export function ServerVerifyBar({ projectId, onError }: { projectId: string; onError: (e: unknown) => void }) {
  const { t } = useLocale();
  const remote = useIntakeRemote();
  const isAdmin = remote.role === "admin";
  const [state, setState] = useState<{ projectId: string; settings: VerifySettings | null } | null>(null);
  const [busy, setBusy] = useState(false);
  const [denyOpen, setDenyOpen] = useState(false);

  useEffect(() => {
    let alive = true;
    getVerify(projectId)
      .then((s) => alive && setState({ projectId, settings: s }))
      .catch(() => alive && setState({ projectId, settings: null }));
    return () => {
      alive = false;
    };
  }, [projectId]);

  // 上一個專案的狀態不能拿來畫這個專案。
  const settings = state?.projectId === projectId ? state.settings : null;
  if (!settings) return null;

  const toggle = async () => {
    setBusy(true);
    try {
      await setVerify(projectId, !settings.enabled);
      setState({ projectId, settings: { ...settings, enabled: !settings.enabled } });
    } catch (e) {
      onError(e);
    } finally {
      setBusy(false);
    }
  };

  const canToggle = isAdmin && (settings.has_verifier || settings.enabled);
  const hint = !settings.has_verifier ? t.ac_server_verify_no_verifier : !isAdmin ? t.ac_server_verify_admin_only : t.ac_server_verify_hint;

  return (
    <>
      <span className="ac-top-sep" aria-hidden />
      <button
        className={`aiterm-btn aiterm-btn--sm ac-top-btn${settings.enabled ? " aiterm-btn--primary" : ""}`}
        data-testid="ac-server-verify"
        title={hint}
        disabled={busy || !canToggle}
        aria-pressed={settings.enabled}
        onClick={() => void toggle()}
      >
        <span aria-hidden>🔍</span>
        <span className="ac-sr-long">
          {t.ac_server_verify} · {settings.enabled ? t.ac_server_run_on : t.ac_server_run_off}
        </span>
        <span className="ac-sr-short">{settings.enabled ? t.ac_server_run_on : t.ac_server_run_off}</span>
      </button>
      {isAdmin && (
        <button className="aiterm-btn aiterm-btn--sm ac-top-btn" data-testid="ac-deny-open" title={t.ac_deny_hint} onClick={() => setDenyOpen(true)}>
          <span aria-hidden>🚫</span>
          <span className="ac-sr-long">{t.ac_deny_title}</span>
        </button>
      )}
      {denyOpen && <DenyListDialog projectId={projectId} onClose={() => setDenyOpen(false)} />}
    </>
  );
}
