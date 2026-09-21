import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { onQuitRequested, setQuitConfirmed, QUIT_REQUESTED_EVENT } from "../ipc/quit";
import { unlistenOnCleanup } from "../lib/eventSubscription";
import type { BusyTab } from "../lib/busyProbe";

/**
 * 攔截整個視窗的關閉。兩個入口——原生關閉（✕／Alt+F4，`onCloseRequested`）與
 * 後端攔下的 Cmd+Q（`app://quit-requested`）——共用同一個 `attempt()`：
 * 沒有忙碌分頁就直接退出，否則交給呼叫端顯示確認框（`pending`）。
 *
 * 原生關閉一律 `preventDefault()` 再自己 `destroy()`：Tauri 預設在 handler 結束後
 * 也會 destroy，兩條路徑各關一次沒有意義，統一由 `quit()` 負責。
 */
export function useWindowCloseGuard(getBusyTabs: () => BusyTab[]) {
  const [pending, setPending] = useState<BusyTab[] | null>(null);
  const pendingRef = useRef<BusyTab[] | null>(null);

  // listener 只註冊一次；忙碌狀態要靠 ref 讀最新的，閉包捕捉會靜默放行。
  const getBusyTabsRef = useRef(getBusyTabs);
  useEffect(() => {
    getBusyTabsRef.current = getBusyTabs;
  });

  const quit = useCallback(async () => {
    // 必須在 destroy 之前：最後一個視窗關閉會再觸發 ExitRequested，後端靠旗標放行。
    try {
      await setQuitConfirmed();
    } catch (e) {
      console.error("set_quit_confirmed 失敗:", e);
    }
    try {
      await getCurrentWindow().destroy();
    } catch (e) {
      console.error("destroy 視窗失敗:", e);
    }
  }, []);

  const attempt = useCallback(async () => {
    if (pendingRef.current) return; // 確認框已顯示，連按 ✕ 不重複處理
    const busy = getBusyTabsRef.current();
    if (busy.length === 0) {
      await quit();
      return;
    }
    pendingRef.current = busy;
    setPending(busy);
  }, [quit]);

  useEffect(() => {
    const unCloseRequested = unlistenOnCleanup(
      getCurrentWindow().onCloseRequested((event) => {
        event.preventDefault();
        void attempt();
      }),
      "tauri://close-requested",
    );
    const unQuitRequested = unlistenOnCleanup(
      onQuitRequested(() => { void attempt(); }),
      QUIT_REQUESTED_EVENT,
    );
    return () => {
      unCloseRequested();
      unQuitRequested();
    };
  }, [attempt]);

  const confirm = useCallback(() => {
    // 不清 pending：視窗即將消失，清掉只會讓連按 ✕ 又跳出第二個框。
    void quit();
  }, [quit]);

  const cancel = useCallback(() => {
    pendingRef.current = null;
    setPending(null);
  }, []);

  return { pending, confirm, cancel };
}
