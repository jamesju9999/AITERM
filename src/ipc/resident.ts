import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 與 Rust `resident::FOCUS_INPUT_EVENT` 相同：視窗被快捷鍵／托盤喚出後送出。 */
export const FOCUS_INPUT_EVENT = "app://focus-input";
/** 與 Rust `resident::NEW_TAB_EVENT` 相同：托盤選單「New Tab」。 */
export const TRAY_NEW_TAB_EVENT = "app://tray-new-tab";

export function onFocusInput(cb: () => void): Promise<UnlistenFn> {
  return listen(FOCUS_INPUT_EVENT, () => cb());
}

export function onTrayNewTab(cb: () => void): Promise<UnlistenFn> {
  return listen(TRAY_NEW_TAB_EVENT, () => cb());
}

/** 視窗 ✕：後端能縮到托盤就隱藏並回傳 true；否則（或呼叫失敗）回傳 false，走原本的結束流程。 */
export async function hideMainWindowForClose(): Promise<boolean> {
  try {
    return await invoke<boolean>("hide_main_window_for_close");
  } catch (e) {
    console.error("hide_main_window_for_close 失敗:", e);
    return false;
  }
}
