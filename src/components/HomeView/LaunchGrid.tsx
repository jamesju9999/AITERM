import { useEffect, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { visibleTabCatalog } from "../NewTabPicker/tabCatalog";
import type { TabType } from "../TabBar";
import { bridgeStatus } from "../../ipc/bridge";

interface Props {
  onOpenTab: (type: TabType, opts?: { claudeBridge?: boolean }) => void;
}

export function LaunchGrid({ onOpenTab }: Props) {
  const { t } = useLocale();

  // 選單看到什麼，首頁就該看到什麼——Claude Code 那一項要看橋接 server
  // 是否在跑，沒在跑時停用。
  const [bridgeRunning, setBridgeRunning] = useState(false);
  useEffect(() => {
    bridgeStatus().then((s) => setBridgeRunning(s.running)).catch(() => {});
  }, []);

  return (
    <section className="home-section">
      <h2 className="home-section-title">{t.home_launch_title}</h2>
      <div className="home-launch-grid">
        {visibleTabCatalog(t).map((entry) => {
          const disabled = !!entry.requiresBridge && !bridgeRunning;
          return (
            <button
              key={entry.id}
              className="home-launch-card"
              disabled={disabled}
              title={disabled ? t.bridge_new_tab_disabled_hint : undefined}
              onClick={() => onOpenTab(entry.type, entry.opts)}
            >
              <span className="home-launch-icon">{entry.icon}</span>
              <span className="home-launch-label">{entry.label}</span>
              <span className="home-launch-desc">
                {disabled ? t.bridge_new_tab_disabled_hint : entry.desc}
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
