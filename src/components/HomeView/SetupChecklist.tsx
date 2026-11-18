import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { getConfig } from "../../ipc/config";
import { OpenSettingsButton } from "../OpenSettingsButton";
import { SectionTitle } from "./SectionTitle";
import { SettingsIcon } from "../Icons";
import {
  buildChecklist, loadDismissed, saveDismissed,
  type ChecklistId, type ChecklistItem,
} from "./checklistLogic";

/**
 * 首頁的設定清單：把「還沒設定」的功能列出來，每列一個按鈕直接跳到對應的設定分頁。
 * 全部設好（或被標成「不需要」）就整塊不顯示，不佔版面。
 *
 * 只讀 get_config（純設定檔，不碰鑰匙圈）——首頁一開就會載入，不能因此跳出系統密碼框。
 */
export function SetupChecklist() {
  const { t } = useLocale();
  const [items, setItems] = useState<ChecklistItem[] | null>(null);
  const [dismissed, setDismissed] = useState<Set<string>>(() => loadDismissed());

  useEffect(() => {
    let cancelled = false;
    getConfig()
      .then((cfg) => {
        if (cancelled) return;
        setItems(buildChecklist({
          providers: cfg.providers?.length ?? 0,
          mailAccounts: cfg.mail_accounts ?? [],
          mcpServers: cfg.mcp_servers?.length ?? 0,
          dbConnections: cfg.db_connections?.length ?? 0,
          vcsConnections: cfg.vcs_connections?.length ?? 0,
          bridgeEnabled: !!cfg.claude_bridge?.enabled,
        }));
      })
      // 讀不到就不顯示清單，也不要因此讓首頁壞掉。
      .catch(() => { if (!cancelled) setItems(null); });
    return () => { cancelled = true; };
  }, []);

  if (!items) return null;

  const pending = items.filter((i) => i.status !== "done" && (i.required || !dismissed.has(i.id)));
  if (pending.length === 0) return null;

  const dismiss = (id: ChecklistId) => {
    const next = new Set(dismissed).add(id);
    setDismissed(next);
    saveDismissed(next);
  };

  const label = (id: ChecklistId) => t[`home_setup_${id}` as const];
  const hint = (i: ChecklistItem) =>
    i.status === "warn" ? t.home_setup_mail_off : t[`home_setup_${i.id}_hint` as const];
  const doneCount = items.filter((i) => i.status === "done").length;

  return (
    <section className="home-section" data-testid="home-setup">
      <SectionTitle icon={<SettingsIcon size={17} />} count={`${doneCount}/${items.length}`}>
        {t.home_setup_title}
      </SectionTitle>
      <ul className="home-setup-list">
        {pending.map((i) => (
          <li key={i.id} className={`home-setup-row home-setup-row--${i.status}`}>
            <span className="home-setup-dot" aria-hidden="true">{i.status === "warn" ? "!" : ""}</span>
            <div className="home-setup-text">
              <div className="home-setup-label">{label(i.id)}</div>
              <div className="home-setup-hint">{hint(i)}</div>
            </div>
            <OpenSettingsButton tab={i.tab} label={t.home_setup_go} />
            {!i.required && (
              <button type="button" className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => dismiss(i.id)}>
                {t.home_setup_dismiss}
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}
