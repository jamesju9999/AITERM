import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 與 Rust `quit::QUIT_REQUESTED_EVENT` 相同。 */
export const QUIT_REQUESTED_EVENT = "app://quit-requested";

/**
 * 通知後端「可以退出了」。必須在 `destroy()` 視窗之前呼叫：最後一個視窗關閉
 * 會再觸發一次 ExitRequested，後端靠這個旗標放行。
 */
export function setQuitConfirmed(): Promise<void> {
  return invoke<void>("set_quit_confirmed");
}

/** 後端攔下使用者的退出請求（macOS Cmd+Q）時觸發，事件不帶資料。 */
export function onQuitRequested(cb: () => void): Promise<UnlistenFn> {
  return listen(QUIT_REQUESTED_EVENT, () => cb());
}
