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

/**
 * 把 ChatGPT 的 webview 顯示出來讓使用者登入。
 *
 * 一定要先叫這個再去讀模型：`chatgptWebModels` 走的是隱藏視窗，未登入時只會
 * 回 `not_logged_in`，使用者不會看到任何可以登入的介面。登入完成後注入腳本的
 * watchLogin 會通知後端把視窗收起來。
 */
export const chatgptWebLogin = () => invoke<void>("chatgpt_web_login");
