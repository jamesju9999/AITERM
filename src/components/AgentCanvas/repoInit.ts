import { confirm } from "@tauri-apps/plugin-dialog";

import type { useLocale } from "../../contexts/LocaleContext";
import { initRepo, type RepoCheck } from "../../ipc/agents";
import { restoreWebviewFocus } from "../../lib/webviewFocus";

/** 問過使用者再一鍵初始化。回傳錯誤訊息；使用者取消或成功都是 null。 */
export async function confirmInitRepo(
  t: ReturnType<typeof useLocale>["t"],
  path: string,
  status: RepoCheck["status"] | undefined,
  projectId: string,
): Promise<string | null> {
  const msg = status === "no_commits" ? t.ac_repo_first_commit_confirm(path) : t.ac_repo_init_confirm(path);
  const ok = await confirm(msg, { kind: "info" });
  // 關掉確認視窗後，網頁檢視要拿回鍵盤焦點，不然文字框點得到卻打不了字。
  await restoreWebviewFocus();
  if (!ok) return null;
  try {
    await initRepo(path, projectId);
    return null;
  } catch (e) {
    return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
  }
}
