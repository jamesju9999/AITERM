import { invoke } from "@tauri-apps/api/core";

export function openUrl(url: string): Promise<void> {
  return invoke("open_url", { url });
}
