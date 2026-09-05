import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";

import { unlistenOnCleanup } from "../../lib/eventSubscription";
import { tryUpgradeTranscript } from "./transcriptUpgrade";

interface TaskFinishedPayload {
  project_id: string;
  task_id: string;
  tab_id: string;
}

/**
 * 訂閱後端的 `task-finished`，把剛完成那張卡片的對話記錄換成 xterm
 * 序列化出來的乾淨版本。
 *
 * 這個 hook 必須掛在**永遠存在**的元件上（TerminalApp），不能放在看板
 * 裡面：看板只有在該專案是當前分頁時才掛載，別的專案的卡片完成時根本
 * 沒人在聽，留下的就是後端那份原始 PTY 擷取（Claude Code 的 TUI 外框
 * 會被壓成無法閱讀的一整條）。順帶也修掉一個更早就存在的缺口——使用者
 * 人在終端機分頁時完成的任務，以前同樣不會被乾淨化。
 */
export function useTranscriptUpgrader(): void {
  useEffect(() => {
    const un = listen<TaskFinishedPayload>("task-finished", (e) => {
      const { project_id, task_id, tab_id } = e.payload;
      void tryUpgradeTranscript(project_id, task_id, tab_id);
    });
    return unlistenOnCleanup(un, "task-finished");
  }, []);
}
