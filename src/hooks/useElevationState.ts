import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { elevationStateEvent } from "../ipc/events";

interface ElevationStatePayload {
  elevated: boolean;
}

/**
 * 訂閱後端的提權狀態事件，供 ElevationBadge 顯示/隱藏用。
 *
 * 連同來源 sessionId 一起存，換分頁時直接從「id 對不上」推導出 false，不必
 * 在 effect 裡另外 setState 重置——跟 useProviderQuota 同樣的作法與理由
 * （否則會觸發 react-hooks/set-state-in-effect，也會讓畫面閃過一格屬於
 * 上一個 session 的狀態）。
 */
export function useElevationState(sessionId: string | null): boolean {
  const [state, setState] = useState<{ id: string; elevated: boolean } | null>(null);

  useEffect(() => {
    if (!sessionId) return;
    let unlisten: (() => void) | undefined;
    void listen<ElevationStatePayload>(elevationStateEvent(sessionId), (event) => {
      setState({ id: sessionId, elevated: event.payload.elevated });
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [sessionId]);

  return state?.id === sessionId ? state.elevated : false;
}
