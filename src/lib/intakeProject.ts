import type { IntakeProject } from "../ipc/intake";

function norm(p: string): string {
  return p.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
}

/** `dir` 是不是 `root` 本身或它底下的資料夾（不分大小寫、斜線方向，且要在目錄邊界上）。 */
export function isUnder(dir: string, root: string): boolean {
  const d = norm(dir);
  const r = norm(root);
  return r !== "" && (d === r || d.startsWith(`${r}/`));
}

/** 分頁目錄落在哪個專案的目錄底下就選它（多個時取路徑最長的）；都不是回 `null`。 */
export function projectForCwd(projects: IntakeProject[], cwd: string | null): IntakeProject | null {
  if (!cwd) return null;
  let best: IntakeProject | null = null;
  for (const p of projects) {
    if (p.repo_dir && isUnder(cwd, p.repo_dir) && (!best || p.repo_dir.length > (best.repo_dir ?? "").length)) best = p;
  }
  return best;
}
