import { useLocale } from "../../contexts/LocaleContext";
import type { Tab } from "../TabBar";

interface Props {
  tabs: Tab[];
  onSelectTab: (id: string) => void;
}

export function RunningTasks({ tabs, onSelectTab }: Props) {
  const { t } = useLocale();
  // 刻意不過濾 enterpriseTask：那是企業浮動面板的規則，首頁要顯示所有任務。
  const running = tabs.filter((tab) => tab.agentProgress);
  if (running.length === 0) return null;

  return (
    <section className="home-section">
      <h2 className="home-section-title">{t.home_running_title}</h2>
      <div className="home-running-list">
        {running.map((tab) => {
          const { done, total } = tab.agentProgress!;
          const pct = Math.round((done / Math.max(total, 1)) * 100);
          return (
            <button
              key={tab.id}
              className="home-running-task"
              onClick={() => onSelectTab(tab.id)}
            >
              <span className="home-running-name">{tab.title}</span>
              <span className="home-running-count">{done} / {total}</span>
              <span className="home-running-bar">
                <span className="home-running-fill" style={{ width: `${pct}%` }} />
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
