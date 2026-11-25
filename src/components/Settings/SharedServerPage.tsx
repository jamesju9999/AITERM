import { useLocale } from "../../contexts/LocaleContext";
import { useIntakeRemote } from "../../hooks/useIntakeRemote";
import { ConnectionCard } from "../IntakeView/ConnectionCard";

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
      <p className="intake__meta intake__hint">{t.settings_shared_server_intro}</p>
      <ConnectionCard status={remote} />
    </div>
  );
}
