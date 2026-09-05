import { useLocale } from "../../contexts/LocaleContext";
import type { ProjectInfo } from "../../ipc/projects";

/**
 * 開啟中專案的分頁列。
 *
 * 關鍵語意：`onClose`（分頁上的 ×）**只是把分頁從這一列拿掉**，
 * 不移除專案、不刪任何檔案、也不影響派工——該專案的卡片照樣會被
 * 排程器派出去。要真的移除專案得回專案總覽用那裡的「移除」。
 * 這兩件事視覺上都像「把專案弄掉」，很容易寫錯，
 * ProjectTabBar.test.tsx 有測試把它釘住。
 */
export function ProjectTabBar({
  projects,
  openIds,
  activeId,
  onActivate,
  onClose,
  onOpenOther,
  onBackToList,
}: {
  projects: ProjectInfo[];
  openIds: string[];
  activeId: string | null;
  onActivate: (id: string) => void;
  onClose: (id: string) => void;
  onOpenOther: () => void;
  onBackToList: () => void;
}) {
  const { t } = useLocale();
  const open = openIds
    .map((id) => projects.find((p) => p.id === id))
    .filter((p): p is ProjectInfo => p !== undefined);

  return (
    <div className="project-tabbar">
      <button
        className="project-tabbar-back"
        data-testid="project-tab-back"
        onClick={onBackToList}
      >
        ≡ {t.proj_back_to_list}
      </button>

      <div className="project-tabbar-tabs">
        {open.map((p) => (
          <div
            key={p.id}
            className={`project-tab${p.id === activeId ? " project-tab--active" : ""}${
              p.status !== "ok" ? " project-tab--broken" : ""
            }`}
          >
            <button
              className="project-tab-label"
              data-testid={`project-tab-${p.id}`}
              onClick={() => onActivate(p.id)}
            >
              {p.counts.running > 0 && (
                <span
                  className="project-tab-running"
                  data-testid={`project-tab-running-${p.id}`}
                >
                  ●
                </span>
              )}
              {p.name}
            </button>
            <button
              className="project-tab-close"
              data-testid={`project-tab-close-${p.id}`}
              title={t.proj_tab_close}
              onClick={(e) => {
                // 不讓點擊冒泡到分頁本體，否則關閉會順帶把這個分頁
                // 設成活躍的，畫面會閃一下已經關掉的專案。
                e.stopPropagation();
                onClose(p.id);
              }}
            >
              ×
            </button>
          </div>
        ))}
      </div>

      <button
        className="project-tabbar-add"
        data-testid="project-tab-add"
        title={t.proj_tab_open_other}
        onClick={onOpenOther}
      >
        +
      </button>
    </div>
  );
}
