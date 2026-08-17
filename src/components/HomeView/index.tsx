import { HomeInput } from "./HomeInput";
import { LaunchGrid } from "./LaunchGrid";
import { RunningTasks } from "./RunningTasks";
import { ResumeSection } from "./ResumeSection";
import { UsageSection } from "./UsageSection";
import type { Tab, TabType } from "../TabBar";
import type { TabOpenOpts } from "../NewTabPicker/tabCatalog";
import type { RouteResult } from "./routeIntent";
import "./index.css";

interface Props {
  onOpenTab: (type: TabType, opts?: TabOpenOpts) => string;
  tabs: Tab[];
  onSelectTab: (id: string) => void;
  /** AI 路由開出了一個分頁（非降級結果）。TerminalApp 用它顯示「猜錯了？換成…」提示。 */
  onAiRouted?: (tabId: string, route: RouteResult) => void;
}

export function HomeView({ onOpenTab, tabs, onSelectTab, onAiRouted }: Props) {
  return (
    <div className="home-view">
      {/* 輸入框是首頁的主角，放在所有區塊之前。 */}
      <HomeInput
        onRoute={(r) => {
          const tabId = onOpenTab(
            r.type,
            r.mission ? { initialMission: { goal: r.mission, maxSteps: 20 } } : undefined,
          );
          // 只有真的是 AI 判斷出來的結果才需要「猜錯了？」提示——降級開出來的
          // 終端機會直接看到任務在跑，本身就說明了發生什麼事。
          if (!r.fallback) onAiRouted?.(tabId, r);
        }}
      />
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
