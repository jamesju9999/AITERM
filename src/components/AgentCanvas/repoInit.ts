import { confirm } from "@tauri-apps/plugin-dialog";

import type { useLocale } from "../../contexts/LocaleContext";
import { initRepo, type RepoCheck } from "../../ipc/agents";

/** 問過使用者再一鍵初始化。回傳錯誤訊息；使用者取消或成功都是 null。 */
export async function confirmInitRepo(
  t: ReturnType<typeof useLocale>["t"],
  path: string,
  status: RepoCheck["status"] | undefined,
  projectId: string,
): Promise<string | null> {
  const msg = status === "no_commits" ? t.ac_repo_first_commit_confirm(path) : t.ac_repo_init_confirm(path);
  if (!(await confirm(msg, { kind: "info" }))) return null;
  try {
    await initRepo(path, projectId);
    return null;
  } catch (e) {
    return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
  }
}
