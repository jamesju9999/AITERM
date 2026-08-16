import { useLocale } from "../contexts/LocaleContext";
import { visibleTabCatalog } from "./NewTabPicker/tabCatalog";
import type { TabType } from "./TabBar";
import "./RouteHint.css";

interface Props {
  /** AI 路由判斷開出來的分頁類型。 */
  pickedType: TabType;
  /** 使用者從下拉選單挑了別種分頁類型。 */
  onPick: (type: TabType) => void;
  onDismiss: () => void;
}

/** 首頁 AI 路由猜錯分頁類型時顯示的反悔提示。只在 RouteResult.fallback
 *  為 false（真的是 AI 判斷的結果，不是降級）時才會被渲染——降級開出來的
 *  終端機本身就在跑任務，不需要再解釋一次。 */
export function RouteHint({ pickedType, onPick, onDismiss }: Props) {
  const { t } = useLocale();
  const catalog = visibleTabCatalog(t);
  const pickedLabel = catalog.find((e) => e.type === pickedType)?.label ?? pickedType;

  // 換成清單：排除目前這一種（選它等於什麼都沒做），並依 type 去重
  // （Claude Code 跟一般終端機的 type 都是 "terminal"）。
  const seen = new Set<string>();
  const switchOptions = catalog.filter(
    (e) => e.type !== pickedType && !seen.has(e.type) && seen.add(e.type),
  );

  return (
    <div className="route-hint">
      <span className="route-hint-text">{t.home_route_hint(pickedLabel)}</span>
      <select
        className="route-hint-select"
        value=""
        onChange={(e) => {
          if (e.target.value) onPick(e.target.value as TabType);
        }}
      >
        <option value="">{t.home_route_switch}</option>
        {switchOptions.map((e) => (
          <option key={e.type} value={e.type}>
            {e.label}
          </option>
        ))}
      </select>
      <button
        type="button"
        className="route-hint-dismiss"
        aria-label={t.home_route_dismiss}
        onClick={onDismiss}
      >
        ✕
      </button>
    </div>
  );
}
