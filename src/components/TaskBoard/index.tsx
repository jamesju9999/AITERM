import { useCallback, useEffect, useRef, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import { listProjects, openProject, type ProjectInfo } from "../../ipc/projects";
import { onTasksUpdated } from "../../ipc/tasks";
import { unlistenOnCleanup } from "../../lib/eventSubscription";
import { ProjectBoard } from "./ProjectBoard";
import { ProjectList } from "./ProjectList";
import { ProjectTabBar } from "./ProjectTabBar";
import "./index.css";

const OPEN_KEY = "aiterm_board_open_projects";
const ACTIVE_KEY = "aiterm_board_active_project";

function readOpenIds(): string[] {
  try {
    const raw = localStorage.getItem(OPEN_KEY);
    const parsed = raw ? (JSON.parse(raw) as unknown) : [];
    return Array.isArray(parsed) ? parsed.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

/**
 * 兩層導覽的路由器。
 *
 * 「開啟中」在這裡純粹是 UI 概念——排程器會派工給**所有**已知專案，
 * 跟分頁列上有沒有它無關（spec D4/D7）。
 */
export function TaskBoardView() {
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [openIds, setOpenIds] = useState<string[]>(readOpenIds);
  const [activeId, setActiveId] = useState<string | null>(
    () => localStorage.getItem(ACTIVE_KEY),
  );
  const [showList, setShowList] = useState(false);

  // 這個元件是覆蓋畫面（TerminalApp 的 `boardActive && <TaskBoardView />`），
  // 使用者一離開就整個卸載，而 refresh() 會被每一次 tasks-updated 觸發——
  // 所以抓完之後要先確認自己還活著才 setState。同 ProjectBoard 的作法。
  const mounted = useRef(true);

  const refresh = useCallback(async () => {
    const rows = await listProjects();
    if (!mounted.current) return;
    setProjects(rows);
  }, []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    const un = onTasksUpdated(() => void refresh());
    const unlisten = unlistenOnCleanup(un, "tasks-updated");
    return () => {
      mounted.current = false;
      unlisten();
    };
  }, [refresh]);

  // 存的是使用者的意圖（未過濾的 openIds/activeId），不是底下算出來的顯示
  // 結果：projects 還沒載進來時 visibleOpenIds 必定是空的，存它等於在每次
  // 啟動的前一瞬間把使用者的分頁清單洗掉。反過來，暫時讀不到的專案（外接
  // 磁碟還沒掛上之類）之後回來時，分頁也就跟著回來。
  useEffect(() => {
    localStorage.setItem(OPEN_KEY, JSON.stringify(openIds));
  }, [openIds]);

  useEffect(() => {
    if (activeId) localStorage.setItem(ACTIVE_KEY, activeId);
    else localStorage.removeItem(ACTIVE_KEY);
  }, [activeId]);

  const activate = useCallback((id: string) => {
    setOpenIds((prev) => (prev.includes(id) ? prev : [...prev, id]));
    setActiveId(id);
    setShowList(false);
  }, []);

  /** 只把分頁從列上拿掉。不移除專案、不刪檔案、不影響派工。
   * 不在這裡挑「下一個活躍分頁」——底下的 `active` 是從 openIds 推導
   * 出來的，關掉目前這個就自動落到剩下的最後一個。 */
  const closeTab = useCallback((id: string) => {
    setOpenIds((prev) => prev.filter((x) => x !== id));
  }, []);

  const openOther = useCallback(async () => {
    const picked = await openFileDialog({
      filters: [{ name: "AITerm 專案", extensions: ["aitprj"] }],
    });
    if (typeof picked !== "string") return;
    const id = await openProject(picked);
    await refresh();
    activate(id);
  }, [activate, refresh]);

  // 分頁列與目前分頁都是**衍生**狀態，在 render 當下算出來，不用 effect
  // ＋ setState 去同步：資料夾可能在上次關閉之後被刪掉，還原回來的 id 不一
  // 定還存在；分頁被關掉之後也要自動落到剩下的最後一個。用 effect 同步會多
  // 跑一次 render，而且中間那一幀畫的是還沒修正的錯誤畫面。
  const visibleOpenIds = openIds.filter((id) => projects.some((p) => p.id === id));
  // 先算出一個確定是 string 的 active，再用它做分支——寫成
  // `active !== null` 的布林旗標的話，TypeScript 不會據此收窄
  // 底下 JSX 中 active 的型別，projectId={active} 會是型別錯誤。
  const active: string | null = visibleOpenIds.includes(activeId ?? "")
    ? activeId
    : (visibleOpenIds[visibleOpenIds.length - 1] ?? null);

  if (showList || active === null) {
    return (
      <div className="task-board">
        <ProjectList projects={projects} onRefresh={refresh} onOpen={activate} />
      </div>
    );
  }

  return (
    <div className="task-board">
      <ProjectTabBar
        projects={projects}
        openIds={visibleOpenIds}
        activeId={active}
        onActivate={activate}
        onClose={closeTab}
        onOpenOther={() => void openOther()}
        onBackToList={() => setShowList(true)}
      />
      {/* key 讓切換專案時整個 ProjectBoard 重新掛載，狀態不會殘留。
          這裡刻意「卸載而非隱藏」——TerminalView 那條「隱藏而非卸載」
          的規則是為了 xterm.js 在無尺寸元素上 resize 會崩潰，
          看板沒有 xterm，不適用。 */}
      <ProjectBoard key={active} projectId={active} />
    </div>
  );
}
