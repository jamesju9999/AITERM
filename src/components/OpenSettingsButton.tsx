import { useInRouterContext, useNavigate } from "react-router-dom";
import { useLocale } from "../contexts/LocaleContext";
import type { Translations } from "../lib/i18n";

/** 設定頁的分頁 id（與 SettingsView 的 SettingsTab 對應）。 */
export type SettingsTarget = "providers" | "mail" | "mcp" | "bridge";

const LABEL_KEY = {
  providers: "open_settings_providers",
  mail: "open_settings_mail",
  mcp: "open_settings_mcp",
  bridge: "open_settings_bridge",
} as const satisfies Record<SettingsTarget, keyof Translations>;

/**
 * 「還沒設定」時放在提示旁邊的跳轉按鈕：一鍵帶使用者到對應的設定分頁。
 * 不在 Router 底下時（例如單獨掛載的測試）不渲染，而不是丟錯。
 */
export function OpenSettingsButton({
  tab,
  onBeforeNavigate,
  className = "aiterm-btn aiterm-btn--secondary aiterm-btn--sm",
}: {
  tab: SettingsTarget;
  /** 跳走之前先收尾，例如關掉對話框。 */
  onBeforeNavigate?: () => void;
  className?: string;
}) {
  const inRouter = useInRouterContext();
  if (!inRouter) return null;
  return <Inner tab={tab} onBeforeNavigate={onBeforeNavigate} className={className} />;
}

function Inner({ tab, onBeforeNavigate, className }: { tab: SettingsTarget; onBeforeNavigate?: () => void; className: string }) {
  const { t } = useLocale();
  const navigate = useNavigate();
  return (
    <button
      type="button"
      className={className}
      onClick={(e) => {
        e.stopPropagation();
        onBeforeNavigate?.();
        navigate("/settings", { state: { tab } });
      }}
    >
      {t[LABEL_KEY[tab]]}
    </button>
  );
}
