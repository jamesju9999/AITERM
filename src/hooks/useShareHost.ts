import { useCallback, useEffect, useRef, useState } from "react";
import {
  onShareViewersChanged,
  shareKick,
  shareRevokeControl,
  shareStart,
  shareStatus,
  shareStop,
  shareViewers,
  type Viewer,
} from "../ipc/share";

/**
 * 一個終端機分頁的分享狀態。
 *
 * **注意這裡沒有任何驗證碼相關的狀態。** 主控端的 4 位碼永遠不離開 Rust
 * ——同意視窗要使用者輸入對方唸的碼，比對在 `share_approve` 裡做。前端
 * 拿不到那個值，所以不可能顯示它。見 `src/ipc/share.ts` 的 `PendingRequest`。
 */
/** `sessionId` 必須是 **PTY session id**，不是 React 的分頁 id——後端拿它
 *  去 `PtyManager` 查串流，傳錯觀看端只會看到「那個終端機已經關閉」。 */
export function useShareHost(sessionId: string) {
  const [sharing, setSharing] = useState(false);
  const [code, setCode] = useState<string | null>(null);
  const [port, setPort] = useState<number | null>(null);
  // 後端還沒送這個欄位（見 2B-2b Task 3 Step 6），執行期會是 undefined——
  // 用 ?? null 收斂成 ShareStatus 宣告的型別。
  const [lanAddress, setLanAddress] = useState<string | null>(null);
  const [viewers, setViewers] = useState<Viewer[]>([]);

  // 事件 callback 只註冊一次，但要讀到最新的 sessionId——用 ref 避免 stale
  // closure（這個 repo 在 Tauri 事件監聽上踩過這個坑）。
  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;

  const refreshViewers = useCallback(async () => {
    const list = await shareViewers(sessionIdRef.current);
    setViewers(list);
  }, []);

  // 掛載時問一次目前狀態——分享是跨分頁切換存活的，重新渲染不該讓面板忘記。
  useEffect(() => {
    let alive = true;
    void shareStatus(sessionId).then((s) => {
      if (!alive) return;
      setSharing(s.sharing);
      setCode(s.code);
      setPort(s.port);
      setLanAddress(s.lanAddress ?? null);
      if (s.sharing) void refreshViewers();
    });
    return () => {
      alive = false;
    };
  }, [sessionId, refreshViewers]);

  // 觀看者變動的推播。事件不帶內容，收到就重讀。
  useEffect(() => {
    let un: (() => void) | null = null;
    let disposed = false;
    void onShareViewersChanged(() => {
      void refreshViewers();
    }).then((f) => {
      if (disposed) f();
      else un = f;
    });
    return () => {
      disposed = true;
      un?.();
    };
  }, [refreshViewers]);

  const start = useCallback(async () => {
    const s = await shareStart(sessionIdRef.current);
    setSharing(s.sharing);
    setCode(s.code);
    setPort(s.port);
    setLanAddress(s.lanAddress ?? null);
  }, []);

  const stop = useCallback(async () => {
    await shareStop(sessionIdRef.current);
    setSharing(false);
    setCode(null);
    setPort(null);
    setLanAddress(null);
    setViewers([]);
  }, []);

  const kick = useCallback(
    async (viewerId: string) => {
      await shareKick(sessionIdRef.current, viewerId);
      await refreshViewers();
    },
    [refreshViewers],
  );

  const revokeControl = useCallback(async () => {
    await shareRevokeControl(sessionIdRef.current);
    await refreshViewers();
  }, [refreshViewers]);

  return { sharing, code, port, lanAddress, viewers, start, stop, kick, revokeControl };
}
