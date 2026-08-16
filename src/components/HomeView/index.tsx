import { LaunchGrid } from "./LaunchGrid";
import { RunningTasks } from "./RunningTasks";
import { ResumeSection } from "./ResumeSection";
import { UsageSection } from "./UsageSection";
import type { Tab, TabType } from "../TabBar";
import type { TabOpenOpts } from "../NewTabPicker/tabCatalog";
import "./index.css";

interface Props {
  onOpenTab: (type: TabType, opts?: TabOpenOpts) => void;
  tabs: Tab[];
  onSelectTab: (id: string) => void;
}

export function HomeView({ onOpenTab, tabs, onSelectTab }: Props) {
  return (
    <div className="home-view">
      {/* 順序：進行中的任務 → 接續上次的工作 → 開始工作 → 今日 AI 用量。
          正在跑的最急，其次是接續昨天的事，再來才是開新的東西，用量是參考資訊。 */}
      <RunningTasks tabs={tabs} onSelectTab={onSelectTab} />
      <ResumeSection
        tabs={tabs}
        onSelectTab={onSelectTab}
        onOpenProject={(path) => onOpenTab("terminal", { initialCwd: path })}
      />
      <LaunchGrid onOpenTab={onOpenTab} />
      <UsageSection />
    </div>
  );
}
