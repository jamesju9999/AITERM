import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Ask AI 要把工作派給 Agent 協作的角色，等使用者確認。後端事件 `canvas://confirm-dispatch`。 */
export interface CanvasConfirmRequest {
  id: string;
  project: string;
  agent: string;
  agent_kind: string;
  title: string;
  instruction: string;
}

export const onCanvasConfirmDispatch = (cb: (r: CanvasConfirmRequest) => void): Promise<UnlistenFn> =>
  listen<CanvasConfirmRequest>("canvas://confirm-dispatch", (e) => cb(e.payload));

/** 回覆確認框。請求已逾時（後端 5 分鐘）回 false。 */
export const canvasConfirmDecide = (id: string, approve: boolean) =>
  invoke<boolean>("canvas_confirm_decide", { id, approve });
