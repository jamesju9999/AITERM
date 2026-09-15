import { invoke } from "@tauri-apps/api/core";

/**
 * 「這台的金鑰不在這台電腦上」。跟 Rust 的
 * `commands::remote_hosts::ERR_SAVED_KEY_MISSING` 必須一字不差。
 *
 * 這是手抄關係，沒有任何編譯期檢查（repo 既有的 `no_remote:` 與 AiError 的
 * kind 標籤也一樣）。
 */
export const ERR_SAVED_KEY_MISSING = "remote_host_key_missing";

/**
 * 「keychain 本身讀不到」——鎖住、權限被拒、資料損毀。
 *
 * **比對一定要用 `startsWith`／`includes`，不能用 `===`。** 後端送出來的形狀是
 * `remote_host_keychain_unavailable: <底層原因>`，用精確比對的話這條分支
 * 永遠不會成立，keychain 故障就會退回顯示一串原始錯誤字串——而把這兩種
 * 錯誤分開的整個用意就沒了。
 */
export const ERR_KEYCHAIN_UNAVAILABLE = "remote_host_keychain_unavailable";

/** 地址簿的一筆，**永遠不含金鑰**。 */
export interface RemoteHostInfo {
  id: string;
  name: string;
  host: string;
  port: number;
  /** 這台電腦的 keychain 裡有沒有它的金鑰。設定檔同步到別台時會是 false。
   *  欄位名維持 snake_case——這個 repo 的 IPC 型別都是（見 src/ipc/vcs.ts 的 has_secret）。 */
  has_key: boolean;
}

export interface RemoteHostInput {
  /** 省略代表新增。 */
  id?: string;
  name: string;
  host: string;
  port: number;
  /** 預共享金鑰。**空字串或省略代表「這次不改金鑰」，不是「清空金鑰」**。 */
  secret?: string;
}

export function remoteHostsList(): Promise<RemoteHostInfo[]> {
  return invoke<RemoteHostInfo[]>("remote_hosts_list");
}

export function remoteHostsAdd(input: RemoteHostInput): Promise<string> {
  return invoke<string>("remote_hosts_add", { input });
}

export function remoteHostsUpdate(input: RemoteHostInput): Promise<void> {
  return invoke<void>("remote_hosts_update", { input });
}

export function remoteHostsRemove(id: string): Promise<void> {
  return invoke<void>("remote_hosts_remove", { id });
}
