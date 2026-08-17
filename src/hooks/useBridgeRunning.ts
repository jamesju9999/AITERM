import { useEffect, useState } from "react";
import { useLocation } from "react-router-dom";
import { bridgeStatus } from "../ipc/bridge";

/**
 * 橋接 server 目前是否在跑。
 *
 * TerminalApp 永遠掛著不會 unmount（見 App.tsx 的說明——PTY session 與 WebGL
 * context 禁不起被摧毀），所以只在 mount 時查一次會在「設定頁啟用/停用橋接
 * → 切回首頁」之後變成 stale 值，沒有任何後續事件會把它更新回來（後端也沒有
 * emit 對應事件）。用 useLocation().pathname 回到 "/"（首頁／終端機視圖）
 * 當作重查的觸發點。
 */
export function useBridgeRunning(): boolean {
  const { pathname } = useLocation();
  const [running, setRunning] = useState(false);

  useEffect(() => {
    if (pathname !== "/") return;
    bridgeStatus().then((s) => setRunning(s.running)).catch(() => {});
  }, [pathname]);

  return running;
}
