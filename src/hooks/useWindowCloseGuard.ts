import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { onQuitRequested, QUIT_REQUESTED_EVENT } from "../ipc/quit";
import { unlistenOnCleanup } from "../lib/eventSubscription";
import type { BusyTab } from "../lib/busyProbe";

/**
 * 攔截整個視窗的關閉。兩個入口——原生關閉（✕／Alt+F4，`onCloseRequested`）與
 * macOS 自訂 Quit 選單項目送來的 `app://quit-requested`（Cmd+Q）——共用同一個
 * `attempt()`：沒有忙碌分頁就直接退出，否則交給呼叫端顯示確認框（`pending`）。
 *
 * Cmd+Q 為什麼要走自訂選單而不是 Rust 的 `RunEvent::ExitRequested`：實測 macOS 預設
 * 選單的 Quit 走原生 `terminate:`，直接進到 `RunEvent::Exit`，`ExitRequested` 根本
 * 不會送出，無從攔截。見 `src-tauri/src/quit.rs`。
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
    // 單一視窗 App：最後一個視窗被 destroy 後，runtime 會自行走退出流程。
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
