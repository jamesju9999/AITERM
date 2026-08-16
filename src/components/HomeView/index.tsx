import { LaunchGrid } from "./LaunchGrid";
import type { TabType } from "../TabBar";
import type { TabOpenOpts } from "../NewTabPicker/tabCatalog";
import "./index.css";

interface Props {
  onOpenTab: (type: TabType, opts?: TabOpenOpts) => void;
}

export function HomeView({ onOpenTab }: Props) {
  return (
    <div className="home-view">
      <LaunchGrid onOpenTab={onOpenTab} />
    </div>
  );
}
