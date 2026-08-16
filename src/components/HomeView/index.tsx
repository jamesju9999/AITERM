import { LaunchGrid } from "./LaunchGrid";
import type { TabType } from "../TabBar";
import "./index.css";

interface Props {
  onOpenTab: (type: TabType, opts?: { claudeBridge?: boolean }) => void;
}

export function HomeView({ onOpenTab }: Props) {
  return (
    <div className="home-view">
      <LaunchGrid onOpenTab={onOpenTab} />
    </div>
  );
}
