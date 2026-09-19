import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** 對應 Rust 端 `launch::LaunchRequest`。 */
export interface LaunchRequest {
  cwd: string | null;
  /** `.command`／`.sh` 檔路徑，要使用者確認才能執行。 */
  script: string | null;
  /** `-e` 之後的 argv。 */
  command: string[] | null;
}

/** 取走並清空後端佇列。 */
export function takeLaunchRequests(): Promise<LaunchRequest[]> {
  return invoke<LaunchRequest[]>("take_launch_requests");
}

/** 後端有新請求入列時觸發（事件本身不帶資料，收到後要自己 `takeLaunchRequests`）。 */
export function onLaunchRequestPending(cb: () => void): Promise<UnlistenFn> {
  return listen("launch-request-pending", () => cb());
}
