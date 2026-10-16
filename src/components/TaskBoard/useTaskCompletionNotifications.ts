import { useEffect } from "react";
import type { RefObject } from "react";
import { listen } from "@tauri-apps/api/event";
import { sendNotification } from "@tauri-apps/plugin-notification";

import { unlistenOnCleanup } from "../../lib/eventSubscription";
import { ensureNotificationPermission } from "../../lib/notifyPermission";
import { getTaskBoardConfig, type TaskOutcome } from "../../ipc/tasks";
import { sendTelegramMessage } from "../../ipc/telegram";

interface TaskFinishedPayload {
  project_id: string;
  task_id: string;
  tab_id: string;
  outcome: TaskOutcome;
  title: string;
  project_name: string;
  error_message: string | null;
}

function desktopBody(p: TaskFinishedPayload): string {
  if (p.outcome === "success") return `${p.project_name} · 完成`;
  if (p.outcome === "cancelled") return `${p.project_name} · 已取消`;
  return `${p.project_name} · 失敗：${p.error_message ?? ""}`;
}

function telegramText(p: TaskFinishedPayload): string {
  const icon = p.outcome === "success" ? "✅" : p.outcome === "cancelled" ? "⏹️" : "❌";
  const line2 =
    p.outcome === "success" ? "完成" :
    p.outcome === "cancelled" ? "已取消" :
    `失敗：${p.error_message ?? ""}`;
  return `${icon} ${p.project_name} — ${p.title}\n${line2}`;
}

/**
 * 派工卡片跑完（成功／失敗／取消）時，依設定分別發桌面通知與/或 Telegram
 * 訊息。掛在永遠存在的元件上（TerminalApp），理由跟 `useAutoCloseFinishedTabs`
 * 完全一樣——看板只有在該專案是當前分頁時才掛載，別的專案完成時沒人在聽。
 *
 * 桌面通知在完成的分頁正是使用者目前正在看的分頁時跳過；Telegram 不看這條
 * 規則，一律發送——人不在電腦前才是它存在的意義。兩條管道各自失敗都不拋出，
 * 不影響對方或卡片本身的完成流程。
 */
export function useTaskCompletionNotifications(activeIdRef: RefObject<string>): void {
  useEffect(() => {
    const un = listen<TaskFinishedPayload>("task-finished", (e) => {
      const payload = e.payload;
      void getTaskBoardConfig().then((cfg) => {
        if (cfg.notify_desktop_on_finish && payload.tab_id !== activeIdRef.current) {
          void ensureNotificationPermission().then((granted) => {
            if (granted) sendNotification({ title: payload.title, body: desktopBody(payload) });
          });
        }
        if (cfg.notify_telegram_on_finish) {
          void sendTelegramMessage(telegramText(payload)).catch(() => {});
        }
      });
    });
    // Agent 協作：同一個模組審查不通過超過上限，流程停下來等人。
    const unAttention = listen<{ project_name: string; module_title: string }>("agent-module-attention", (e) => {
      const { project_name, module_title } = e.payload;
      void getTaskBoardConfig().then((cfg) => {
        const body = `${project_name} · 審查不通過超過上限，需要你處理`;
        if (cfg.notify_desktop_on_finish) {
          void ensureNotificationPermission().then((granted) => {
            if (granted) sendNotification({ title: module_title, body });
          });
        }
        if (cfg.notify_telegram_on_finish) {
          void sendTelegramMessage(`⚠️ ${project_name} — ${module_title}\n審查不通過超過上限，需要你處理`).catch(() => {});
        }
      });
    });
    // 派出去的 Agent 停下來等人回覆（後端 tasks::quota 偵測）。
    const unReply = listen<{ project_name: string; title: string }>("agent-reply-wait", (e) => {
      const { project_name, title } = e.payload;
      void getTaskBoardConfig().then((cfg) => {
        if (cfg.notify_desktop_on_finish) {
          void ensureNotificationPermission().then((granted) => {
            if (granted) sendNotification({ title, body: `${project_name} · 在等你回覆` });
          });
        }
        if (cfg.notify_telegram_on_finish) {
          void sendTelegramMessage(`💬 ${project_name} — ${title}\n在等你回覆`).catch(() => {});
        }
      });
    });
    const stopFinished = unlistenOnCleanup(un, "task-finished");
    const stopAttention = unlistenOnCleanup(unAttention, "agent-module-attention");
    const stopReply = unlistenOnCleanup(unReply, "agent-reply-wait");
    return () => {
      stopFinished();
      stopAttention();
      stopReply();
    };
  }, [activeIdRef]);
}
