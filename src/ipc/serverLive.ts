import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** 一個執行中、可以觀看的瀏覽器（Agent 的某張卡＋系統）。 */
export interface LiveRun {
  card_id: string;
  system_id: string;
  system_name: string;
  agent_id: string | null;
  manual: boolean;
  viewers: number;
}

/** server → 前端的訊息。 */
export type LiveServerMsg =
  | { t: "frame"; data: string; w: number; h: number }
  | { t: "state"; manual: boolean; url: string; can_operate: boolean; mode: "agent" | "login" }
  | { t: "closed"; why: string }
  | { t: "error"; message: string };

/** 前端 → server 的輸入（只有管理員的會被執行）。座標是 0～1 的正規化座標。 */
export type LiveClientMsg =
  | { t: "mouse"; type: "move" | "down" | "up" | "wheel"; x: number; y: number; button?: "left" | "right" | "middle"; dx?: number; dy?: number }
  | { t: "key"; type: "down" | "up"; key: string; code: string; text?: string; vk: number; mods: number }
  | { t: "text"; text: string }
  | { t: "takeover"; on: boolean }
  | { t: "done" }
  | { t: "back" };

export const listLiveRuns = (projectId: string): Promise<LiveRun[]> => invoke("server_live_runs", { projectId });
/** 開始遠端登入（管理員限定）；已經有人在登入時會丟「已經有人在遠端登入」，呼叫端可以直接去看那個畫面。 */
export const startRemoteLogin = (systemId: string): Promise<void> => invoke("server_live_login_start", { systemId });
export const cancelRemoteLogin = (systemId: string): Promise<void> => invoke("server_live_login_cancel", { systemId });

export interface LiveConnection {
  send: (msg: LiveClientMsg) => void;
  close: () => void;
}

/**
 * 連上一個瀏覽器的畫面串流。先註冊監聽再叫後端開始轉送（事件在訂閱之前送出會遺失）。
 * `onMessage` 收 server 的訊息，`onEnd` 在連線結束時呼叫一次。
 */
export async function openLive(
  target: { kind: "agent" | "login"; systemId: string; cardId?: string },
  onMessage: (m: LiveServerMsg) => void,
  onEnd: () => void,
): Promise<LiveConnection> {
  const handle = await invoke<number>("server_live_open", { kind: target.kind, systemId: target.systemId, cardId: target.cardId ?? null });
  let ended = false;
  const finish = () => {
    if (!ended) {
      ended = true;
      onEnd();
    }
  };
  const unMsg = await listen<string>(`live://msg/${handle}`, (e) => {
    try {
      onMessage(JSON.parse(e.payload) as LiveServerMsg);
    } catch {
      // 不認得的訊息直接略過。
    }
  });
  const unEnd = await listen(`live://end/${handle}`, finish);
  await invoke("server_live_start", { handle });
  return {
    send: (msg) => {
      void invoke("server_live_send", { handle, message: JSON.stringify(msg) }).catch(() => {});
    },
    close: () => {
      unMsg();
      unEnd();
      void invoke("server_live_close", { handle }).catch(() => {});
    },
  };
}
