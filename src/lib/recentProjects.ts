export const RECENT_PROJECTS_KEY = "aiterm-recent-projects";
export const MAX_RECENT_PROJECTS = 10;

export interface RecentProject {
  path: string;
  /** 最後一次進到這個目錄的時間（epoch ms）。 */
  lastUsedAt: number;
}

/**
 * 工作看板／Agent 協作派工時開的工作資料夾（`<看板專案>/agents/<id>/worktree`、
 * `<看板專案>/agents/<id>/reviews/<卡片>`、`<看板專案>/tasks/<id>/worktree`，
 * 規則在 src-tauri 的 tasks::task_dir 與 agents::agent_dir）。派工分頁也是終端機，
 * 會回報 cwd，不排除的話「最近的專案目錄」會被一串 worktree 和 UUID 洗掉。
 */
export function isDispatchWorkdir(path: string): boolean {
  const parts = path.split(/[\\/]+/);
  return parts.some(
    (seg, i) =>
      (seg === "agents" && (parts[i + 2] === "worktree" || parts[i + 2] === "reviews")) ||
      (seg === "tasks" && parts[i + 2] === "worktree"),
  );
}

export function listRecentProjects(): RecentProject[] {
  try {
    const raw = localStorage.getItem(RECENT_PROJECTS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    // 舊版已經存進去的工作資料夾也一併濾掉，名額讓給真正的專案。
    return Array.isArray(parsed) ? parsed.filter((p: RecentProject) => !isDispatchWorkdir(p.path)) : [];
  } catch {
    return [];
  }
}

/**
 * 記錄一次「使用了某個專案目錄」。
 *
 * `previousPath` 是這個分頁**上一次**所在的目錄；相同就不記錄。這不是效能
 * 優化，是正確性：`TerminalView` 的 cwd 輪詢每次執行都把它的 `lastSaved`
 * 重設為空字串，所以每個還原的分頁在開機約兩秒後都會回報一次自己原本就在的
 * 目錄。少了這道判斷，開機本身就會把清單洗成「反向的分頁清單」——開著十個
 * 分頁的話，`MAX_RECENT_PROJECTS` 會被一次擠光，昨天用過但今天沒開分頁的
 * 專案永久消失，而那正是這個清單的用途。
 */
export function recordProject(path: string, previousPath?: string) {
  if (!path || path === previousPath || isDispatchWorkdir(path)) return;
  const rest = listRecentProjects().filter((p) => p.path !== path);
  const next = [{ path, lastUsedAt: Date.now() }, ...rest].slice(0, MAX_RECENT_PROJECTS);
  localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(next));
}
