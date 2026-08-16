import { useEffect, useRef, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import type { TabType } from "../TabBar";
import { bridgeStatus } from "../../ipc/bridge";
import { RobotIcon } from "../Icons";
import { visibleTabCatalog } from "./tabCatalog";
import "./index.css";

interface Props {
  /** opts.claudeBridge 只有「新增 Claude Code 分頁」那個選項會帶 true。 */
  onSelect: (type: TabType, opts?: { claudeBridge?: boolean }) => void;
  onClose: () => void;
}

export function NewTabPicker({ onSelect, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const { t } = useLocale();

  // 橋接 server 沒在跑就不給選——建立一個注入了死埠位址的分頁，比不給選更難除錯。
  const [bridgeRunning, setBridgeRunning] = useState(false);
  useEffect(() => {
    bridgeStatus().then((s) => setBridgeRunning(s.running)).catch(() => {});
  }, []);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const handleClick = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    document.addEventListener("keydown", handleKey);
    document.addEventListener("mousedown", handleClick);
    return () => {
      document.removeEventListener("keydown", handleKey);
      document.removeEventListener("mousedown", handleClick);
    };
  }, [onClose]);

  const items = visibleTabCatalog(t);

  return (
    <div className="new-tab-picker" ref={ref}>
      {items.map(({ type, icon, label, desc }) => (
        <button
          key={type}
          className="new-tab-picker__item"
          onClick={() => { onSelect(type); onClose(); }}
        >
          <span className="new-tab-picker__icon">{icon}</span>
          <div>
            <div className="new-tab-picker__label">{label}</div>
            <div className="new-tab-picker__desc">{desc}</div>
          </div>
        </button>
      ))}
      {/* 橋接未啟動時停用——建立一個注入了死埠位址的分頁，比不給選更難除錯。 */}
      <button
        className="new-tab-picker__item"
        disabled={!bridgeRunning}
        title={bridgeRunning ? undefined : t.bridge_new_tab_disabled_hint}
        onClick={() => { onSelect("terminal", { claudeBridge: true }); onClose(); }}
      >
        <span className="new-tab-picker__icon"><RobotIcon size={18} /></span>
        <div>
          <div className="new-tab-picker__label">{t.bridge_new_tab}</div>
          <div className="new-tab-picker__desc">
            {bridgeRunning ? t.bridge_new_tab_desc : t.bridge_new_tab_disabled_hint}
          </div>
        </div>
      </button>
    </div>
  );
}
