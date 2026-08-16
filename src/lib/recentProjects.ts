export const RECENT_PROJECTS_KEY = "aiterm-recent-projects";
export const MAX_RECENT_PROJECTS = 10;

export interface RecentProject {
  path: string;
  /** 最後一次進到這個目錄的時間（epoch ms）。 */
  lastUsedAt: number;
}

export function listRecentProjects(): RecentProject[] {
  try {
    const raw = localStorage.getItem(RECENT_PROJECTS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

export function recordProject(path: string) {
  if (!path) return;
  const rest = listRecentProjects().filter((p) => p.path !== path);
  const next = [{ path, lastUsedAt: Date.now() }, ...rest].slice(0, MAX_RECENT_PROJECTS);
  localStorage.setItem(RECENT_PROJECTS_KEY, JSON.stringify(next));
}
