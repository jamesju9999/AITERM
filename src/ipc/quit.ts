import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 與 Rust `quit::QUIT_REQUESTED_EVENT` 相同。 */
export const QUIT_REQUESTED_EVENT = "app://quit-requested";

/** 使用者按下 macOS 的 Cmd+Q（自訂 Quit 選單項目）時觸發，事件不帶資料。 */
export function onQuitRequested(cb: () => void): Promise<UnlistenFn> {
  return listen(QUIT_REQUESTED_EVENT, () => cb());
}
