import { useCallback, useEffect, useState } from "react";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";

import { listProjects, openProject, type ProjectInfo } from "../../ipc/projects";
import { onTasksUpdated } from "../../ipc/tasks";
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

  const refresh = useCallback(async () => {
    setProjects(await listProjects());
  }, []);

  useEffect(() => {
    void refresh();
    const un = onTasksUpdated(() => void refresh());
    return () => void un.then((f) => f());
  }, [refresh]);

  // 還原時過濾掉已不存在的專案——資料夾可能在上次關閉之後被刪掉了。
  // 等 projects 真的載進來才做，否則第一次 render（projects 還是空陣列）
  // 會把所有分頁都當成不存在而清光。
  useEffect(() => {
    if (projects.length === 0) return;
    const known = new Set(projects.map((p) => p.id));
    setOpenIds((prev) => {
      const next = prev.filter((id) => known.has(id));
      return next.length === prev.length ? prev : next;
    });
  }, [projects]);

  // 讓 activeId 永遠是 openIds 的合法成員之一。這一個 effect 同時處理
  // 兩種情況：(a) 上面那個 effect 把已消失的專案篩掉之後，被還原的
  // activeId 可能已經不在清單裡；(b) 分頁被關閉之後需要選出下一個
  // 活躍分頁——落到剩下的最後一個。兩者共用同一段邏輯，不需要
  // closeTab 自己算一次「剩下的最後一個」。
  useEffect(() => {
    setActiveId((current) => {
      if (current !== null && openIds.includes(current)) return current;
      return openIds.length > 0 ? openIds[openIds.length - 1] : null;
    });
  }, [openIds]);

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
   * activeId 的修正交給上面那個同步 effect 處理——不在這裡直接
   * setActiveId，避免兩個 setState 對 openIds 的認知在同一次事件
   * 處理中不一致。 */
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

  const visibleOpenIds = openIds.filter((id) => projects.some((p) => p.id === id));
  // 先算出一個確定是 string 的 active，再用它做分支——寫成
  // `active !== null` 的布林旗標的話，TypeScript 不會據此收窄
  // 底下 JSX 中 active 的型別，projectId={active} 會是型別錯誤。
  const active = visibleOpenIds.includes(activeId ?? "") ? activeId : null;

  if (showList || active === null) {
    return (
      <div className="task-board">
        <ProjectList onOpen={activate} />
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
