import { LaunchGrid } from "./LaunchGrid";
import { RunningTasks } from "./RunningTasks";
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
      {/* 進行中的任務比入口更該先看到，放在 LaunchGrid 之前。 */}
      <RunningTasks tabs={tabs} onSelectTab={onSelectTab} />
      <LaunchGrid onOpenTab={onOpenTab} />
    </div>
  );
}
