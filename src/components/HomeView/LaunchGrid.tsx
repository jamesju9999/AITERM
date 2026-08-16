import { useLocale } from "../../contexts/LocaleContext";
import { visibleTabCatalog } from "../NewTabPicker/tabCatalog";
import type { TabType } from "../TabBar";

interface Props {
  onOpenTab: (type: TabType) => void;
}

export function LaunchGrid({ onOpenTab }: Props) {
  const { t } = useLocale();
  return (
    <section className="home-section">
      <h2 className="home-section-title">{t.home_launch_title}</h2>
      <div className="home-launch-grid">
        {visibleTabCatalog(t).map((entry) => (
          <button
            key={entry.type}
            className="home-launch-card"
            onClick={() => onOpenTab(entry.type)}
          >
            <span className="home-launch-icon">{entry.icon}</span>
            <span className="home-launch-label">{entry.label}</span>
            <span className="home-launch-desc">{entry.desc}</span>
          </button>
        ))}
      </div>
    </section>
  );
}
