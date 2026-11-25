import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { getOrchestration, setOrchestration } from "../../ipc/tasks";

/**
 * 共用看板專案的「由 server 執行」開關。
 * 只在共用專案出現；本機專案（getOrchestration 回 null）什麼都不畫。
 * 只有管理員能開關，不是管理員時由 server 拒絕，原因交給 onError 顯示。核准合併在 `ServerMergeBanner`。
 */
export function ServerRunBar({
  projectId,
  onError,
}: {
  projectId: string;
  onError: (e: unknown) => void;
}) {
  const { t } = useLocale();
  const [state, setState] = useState<{ projectId: string; on: boolean | null } | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let alive = true;
    getOrchestration(projectId)
      .then((on) => alive && setState({ projectId, on }))
      .catch(() => alive && setState({ projectId, on: null }));
    return () => {
      alive = false;
    };
  }, [projectId]);

  // 上一個專案的狀態不能拿來畫這個專案。
  const on = state?.projectId === projectId ? state.on : null;
  if (on === null) return null;

  const toggle = async () => {
    setBusy(true);
    try {
      await setOrchestration(projectId, !on);
      setState({ projectId, on: !on });
    } catch (e) {
      onError(e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <span className="ac-top-sep" aria-hidden />
      <button
        className={`aiterm-btn aiterm-btn--sm ac-top-btn${on ? " aiterm-btn--primary" : ""}`}
        data-testid="ac-server-run"
        title={t.ac_server_run_hint}
        disabled={busy}
        aria-pressed={on}
        onClick={() => void toggle()}
      >
        <span aria-hidden>☁</span>
        {/* 頂列很擠：寬視窗顯示完整文字，窄視窗只留狀態字（不能像別的按鈕只留圖示，不然看不出開關是開是關）。 */}
        <span className="ac-sr-long">
          {t.ac_server_run} · {on ? t.ac_server_run_on : t.ac_server_run_off}
        </span>
        <span className="ac-sr-short">{on ? t.ac_server_run_on : t.ac_server_run_off}</span>
      </button>
    </>
  );
}
