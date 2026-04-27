import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface TelegramConfigData {
  bot_token: string | null;
  chat_id: string | null;
}

export interface TelegramMessagePayload {
  text: string;
}

export async function getTelegramConfig(): Promise<TelegramConfigData> {
  return await invoke("telegram_get_config");
}

export async function setTelegramConfig(config: TelegramConfigData): Promise<void> {
  await invoke("telegram_set_config", { config });
}

export async function sendTelegramMessage(text: string): Promise<void> {
  await invoke("telegram_send_message", { text });
}

export async function listenTelegramMessage(
  callback: (payload: TelegramMessagePayload) => void
): Promise<() => void> {
  const unlisten = await listen<TelegramMessagePayload>("telegram-message-received", (event) => {
    callback(event.payload);
  });
  return unlisten;
}
