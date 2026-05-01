import { invoke } from "@tauri-apps/api/core";

export async function webSearch(query: string): Promise<string> {
  return invoke<string>("web_search", { query });
}

export async function webFetch(url: string): Promise<string> {
  return invoke<string>("web_fetch", { url });
}
