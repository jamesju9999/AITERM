import { useEffect, useRef } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { OpenSettingsButton } from "../OpenSettingsButton";
import type { TabType } from "../TabBar";
import { useBridgeRunning } from "../../hooks/useBridgeRunning";
import { groupedVisibleTabCatalog, type TabOpenOpts } from "./tabCatalog";
import "./index.css";

interface Props {
  /** opts.claudeBridge 只有「新增 Claude Code 分頁」那個選項會帶 true。 */
  onSelect: (type: TabType, opts?: TabOpenOpts) => void;
  onClose: () => void;
}

export function NewTabPicker({ onSelect, onClose }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const { t } = useLocale();

  // 橋接 server 沒在跑就不給選——建立一個注入了死埠位址的分頁，比不給選更難除錯。
  const bridgeRunning = useBridgeRunning();

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

  const groups = groupedVisibleTabCatalog(t);
  const groupLabel = { ai: t.tab_group_ai, data: t.tab_group_data, workflow: t.tab_group_workflow } as const;

  return (
    <div className="new-tab-picker" ref={ref}>
      {groups.map(({ group, entries }) => {
        // 橋接未啟動時停用——建立一個注入了死埠位址的分頁，比不給選更難除錯。
        const isDisabled = (e: (typeof entries)[number]) => !!e.requiresBridge && !bridgeRunning;
        // 停用的項目要帶原因與跳轉按鈕，橫跨兩欄放在該組最後；其餘項目若是奇數個，
        // 最後一個也橫跨兩欄，這樣格子不會留一個空洞。
        const normal = entries.filter((e) => !isDisabled(e));
        const wide = entries.filter(isDisabled);
        const lastNormalId = normal.length % 2 === 1 ? normal[normal.length - 1].id : null;
        return (
        <section key={group} className="new-tab-picker__group">
          <div className="new-tab-picker__group-title">{groupLabel[group]}</div>
          <div className="new-tab-picker__grid">
            {[...normal, ...wide].map((entry) => {
              const disabled = isDisabled(entry);
              const cls = `new-tab-picker__row${disabled ? " new-tab-picker__row--wide" : ""}${entry.id === lastNormalId ? " new-tab-picker__row--span" : ""}`;
              return (
                <div key={entry.id} className={cls} style={{ ["--tab-color" as string]: entry.color }}>
                  <button
                    className="new-tab-picker__item"
                    disabled={disabled}
                    // 說明文字改放在提示裡，兩欄才排得下；停用時直接顯示原因。
                    title={disabled ? entry.disabledHint : entry.desc}
                    onClick={() => {
                      onSelect(entry.type, entry.opts);
                      onClose();
                    }}
                  >
                    <span className="new-tab-picker__icon">{entry.icon}</span>
                    <div>
                      <div className="new-tab-picker__label">{entry.label}</div>
                      {disabled && <div className="new-tab-picker__desc">{entry.disabledHint}</div>}
                    </div>
                  </button>
                  {disabled && (
                    <OpenSettingsButton
                      tab="bridge"
                      onBeforeNavigate={onClose}
                      className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm new-tab-picker__goto"
                    />
                  )}
                </div>
              );
            })}
          </div>
        </section>
        );
      })}
    </div>
  );
}
