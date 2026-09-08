import { useEffect } from "react";
import type { RefObject } from "react";
import { listen } from "@tauri-apps/api/event";

import { unlistenOnCleanup } from "../../lib/eventSubscription";
import { getTaskBoardConfig } from "../../ipc/tasks";

interface TaskFinishedPayload {
  project_id: string;
  task_id: string;
  tab_id: string;
  outcome: string;
}

/**
 * 派工卡片跑完後，若不是使用者目前正在看的分頁、結局不是失敗、且設定
 * 開著，就 dispatch 既有的 `aiterm:close-tab`（TaskCard 刪除卡片時已經
 * 在用的同一個事件）——不新增任何關閉分頁/砍行程的邏輯，只是在對的
 * 時機觸發已經存在、已經驗證過的路徑（見 `TerminalApp.tsx` 的
 * `aiterm:close-tab` 監聽器與 `TerminalView` 卸載時的 `closePty()`）。
 *
 * 必須掛在**永遠存在**的元件上（TerminalApp），理由跟 `useTranscriptUpgrader`
 * 完全一樣——看板只有在該專案是當前分頁時才掛載，別的專案完成時沒人在聽。
 *
 * `activeIdRef` 由呼叫端傳入並在事件觸發當下讀 `.current`，避免閉包
 * 捕捉到掛載當時的舊值。
 */
export function useAutoCloseFinishedTabs(activeIdRef: RefObject<string>): void {
  useEffect(() => {
    const un = listen<TaskFinishedPayload>("task-finished", (e) => {
      const { tab_id, outcome } = e.payload;
      if (outcome === "failed") return;
      if (tab_id === activeIdRef.current) return;
      void getTaskBoardConfig().then((cfg) => {
        if (!cfg.auto_close_finished_tabs) return;
        // skipGuard: true — 這是背景自動關閉，沒有人在看著會去按確認框。
        // isBusyRef/missionActiveRef/isRunningTaskTab 這三個 close guard 依賴
        // 的前端訊號，都是跟後端這個 task-finished 事件完全獨立、非同步的
        // 訊號來源，天生就可能還沒跟上（尤其 isRunningTaskTab 依賴
        // TaskBoardView 自己的 tasks-updated 刷新）——若照常經過 guard，
        // 輕則分頁關不掉、重則 runCloseGuard 會先把使用者正在看的分頁搶走
        // 去顯示那個永遠沒人會點的確認框。後端的 outcome（已經排除
        // failed）本來就是比這三個前端訊號更權威的判斷，不需要再讓前端
        // 重新猜一次。
        window.dispatchEvent(
          new CustomEvent("aiterm:close-tab", { detail: { tabId: tab_id, skipGuard: true } }),
        );
      });
    });
    return unlistenOnCleanup(un, "task-finished");
  }, [activeIdRef]);
}
