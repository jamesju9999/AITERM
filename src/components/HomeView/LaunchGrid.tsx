import { useLocale } from "../../contexts/LocaleContext";
import { visibleTabCatalog, type TabOpenOpts } from "../NewTabPicker/tabCatalog";
import type { TabType } from "../TabBar";
import { useBridgeRunning } from "../../hooks/useBridgeRunning";
import { SectionTitle } from "./SectionTitle";
import { ZapIcon } from "../Icons";

interface Props {
  onOpenTab: (type: TabType, opts?: TabOpenOpts) => void;
}

export function LaunchGrid({ onOpenTab }: Props) {
  const { t } = useLocale();

  // 選單看到什麼，首頁就該看到什麼——Claude Code 那一項要看橋接 server
  // 是否在跑，沒在跑時停用。
  const bridgeRunning = useBridgeRunning();

  return (
    <section className="home-section">
      <SectionTitle icon={<ZapIcon size={17} />}>{t.home_launch_title}</SectionTitle>
      <div className="home-launch-grid">
        {visibleTabCatalog(t).map((entry) => {
          const disabled = !!entry.requiresBridge && !bridgeRunning;
          return (
            <button
              key={entry.id}
              className="home-launch-card"
              disabled={disabled}
              title={disabled ? entry.disabledHint : undefined}
              onClick={() => onOpenTab(entry.type, entry.opts)}
            >
              <span className="home-launch-icon">{entry.icon}</span>
              <span className="home-launch-label">{entry.label}</span>
              <span className="home-launch-desc">
                {disabled ? entry.disabledHint : entry.desc}
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
