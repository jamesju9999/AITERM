import { invoke } from "@tauri-apps/api/core";

export function openUrl(url: string): Promise<void> {
  return invoke("open_url", { url });
}

/** 已安裝但不在 AITerm 行程 PATH 上的 PowerShell 7 路徑，沒有就回 null。 */
export function detectPowerShell7(): Promise<string | null> {
  return invoke<string | null>("detect_powershell7");
}
