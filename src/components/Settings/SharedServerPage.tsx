import { useLocale } from "../../contexts/LocaleContext";
import { useIntakeRemote } from "../../hooks/useIntakeRemote";
import "./SharedServerPage.css";
import { ConnectionCard } from "../IntakeView/ConnectionCard";
import { SavedServers } from "../IntakeView/SavedServers";

/**
 * 共用 server 的連線設定。連線是整個 AITerm 共用的（問題收集、工作看板、Agent 協作都走同一條），
 * 所以入口放在設定，而不是藏在某一個功能裡。
 */
export function SharedServerPage() {
  const { t } = useLocale();
  const remote = useIntakeRemote();
  return (
    <div className="shared-server-page">
      <h2>{t.settings_tab_sharedServer}</h2>
      <p className="shared-server-intro">{t.settings_shared_server_intro}</p>
      {/* 卡片裡的欄位樣式掛在 .intake__settings 底下（問題收集頁的設定面板），這裡沿用同一套。 */}
      <div className="intake__settings">
        <ConnectionCard status={remote} />
        <SavedServers status={remote} />
      </div>
    </div>
  );
}
