import { invoke } from "@tauri-apps/api/core";

export interface BridgeStatus {
  running: boolean;
  port: number | null;
  /** 已產生的 token，供「複製手動命令」使用。未啟動時為 null。 */
  token: string | null;
  /** 啟動失敗的原因（例如埠被占用）。這是使用者要處理的狀態，不是例外。 */
  error: string | null;
}

export function bridgeStatus(): Promise<BridgeStatus> {
  return invoke<BridgeStatus>("bridge_status");
}

/** 依目前 config 啟動或停止 server。設定存檔後呼叫。 */
export function bridgeApply(): Promise<BridgeStatus> {
  return invoke<BridgeStatus>("bridge_apply");
}
