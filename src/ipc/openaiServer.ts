import { invoke } from "@tauri-apps/api/core";

export interface OpenAiServerStatus {
  running: boolean;
  port: number | null;
  /** 目前是否綁在全介面（區網可連）。 */
  lan: boolean;
  /** 給外部工具使用的 API key。未啟用時為 null。 */
  token: string | null;
  /** 啟動失敗的原因（例如埠被占用）。這是使用者要處理的狀態，不是例外。 */
  error: string | null;
}

export interface ModelAlias {
  alias: string;
  provider_id: string;
  model: string;
}

/** 欄位名用 snake_case：Rust 端 serde 沒有 rename_all，跟 `OpenAiServerStatus`（camelCase）不同。 */
export interface OpenAiServerConfig {
  enabled: boolean;
  port: number;
  allow_lan: boolean;
  aliases: ModelAlias[];
}

export function openaiServerStatus(): Promise<OpenAiServerStatus> {
  return invoke<OpenAiServerStatus>("openai_server_status");
}

/** 驗證失敗（別名重複、埠不合法…）時 reject 一個字串，直接顯示給使用者。 */
export function openaiServerSetConfig(value: OpenAiServerConfig): Promise<OpenAiServerStatus> {
  return invoke<OpenAiServerStatus>("openai_server_set_config", { value });
}

/** 產生新的 API key，舊 key 立即失效。 */
export function openaiServerRegenerateKey(): Promise<OpenAiServerStatus> {
  return invoke<OpenAiServerStatus>("openai_server_regenerate_key");
}
