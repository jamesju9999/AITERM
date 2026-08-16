import { useLocale } from "../../contexts/LocaleContext";
import { listRecentProjects } from "../../lib/recentProjects";
import type { Tab } from "../TabBar";

interface Props {
  tabs: Tab[];
  onSelectTab: (id: string) => void;
  onOpenProject: (path: string) => void;
}

export function ResumeSection({ tabs, onSelectTab, onOpenProject }: Props) {
  const { t } = useLocale();
  // 首頁每次顯示都重讀：清單只在 cwd 變動時寫入，不需要訂閱機制。
  const projects = listRecentProjects();

  if (tabs.length === 0 && projects.length === 0) {
    return (
      <section className="home-section">
        <h2 className="home-section-title">{t.home_resume_title}</h2>
        <p className="home-empty">{t.home_resume_empty}</p>
      </section>
    );
  }

  return (
    <section className="home-section">
      <h2 className="home-section-title">{t.home_resume_title}</h2>

      <div className="home-resume-grid">
        {tabs.map((tab) => (
          <button key={tab.id} className="home-resume-card" onClick={() => onSelectTab(tab.id)}>
            <span className="home-resume-title">{tab.title}</span>
            {tab.cwd && <span className="home-resume-cwd">{tab.cwd}</span>}
            {tab.aiSummary && <span className="home-resume-summary">{tab.aiSummary}</span>}
          </button>
        ))}
      </div>

      {projects.length > 0 && (
        <>
          <h3 className="home-subsection-title">{t.home_recent_projects}</h3>
          <div className="home-recent-list">
            {projects.map((p) => (
              <button key={p.path} className="home-recent-item" onClick={() => onOpenProject(p.path)}>
                {p.path}
              </button>
            ))}
          </div>
        </>
      )}
    </section>
  );
}
