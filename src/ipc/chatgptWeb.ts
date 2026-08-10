import { invoke } from "@tauri-apps/api/core";

export interface ChatgptWebModel {
  slug: string;
  title: string;
  max_tokens: number;
}

/**
 * 取回該帳號實際可用的模型清單。
 *
 * 走的是隱藏 webview 裡的 `/backend-api/models`，所以不需要維護「方案 → 模型」
 * 的對應表：登入哪個帳號就顯示什麼。尚未登入時會回錯誤字串。
 */
export const chatgptWebModels = () => invoke<ChatgptWebModel[]>("chatgpt_web_models");
