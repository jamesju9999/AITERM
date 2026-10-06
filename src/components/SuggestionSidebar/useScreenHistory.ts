import { useCallback, useEffect, useRef } from "react";
import { pushScreen } from "../../lib/screenHistory";
import { serializeTerminal } from "../../lib/terminalInstanceRegistry";
import { IDLE_MS, POLL_MS } from "./terminalIdle";

/**
 * 記錄 AI 命令列工具每一輪回完話之後的「穩定畫面」，讓建議能看到「之前做了什麼」，
 * 不只是目前那一屏。
 *
 * - 掛在 TerminalView，**不是**側欄元件：側欄關著也要持續記錄，否則使用者一開側欄，
 *   前面的歷史就已經丟了。
 * - 「穩定」＝終端機從忙碌轉為閒置（輸出安靜 IDLE_MS）。TUI 會一直重畫旋轉動畫與
 *   進度行，讀整份捲動歷史只會得到一堆重複畫面幀；閒置時的那一屏才是這一輪的結論。
 * - 只放記憶體（ref），不寫磁碟：畫面可能含機敏內容。工具結束或換分頁就清空。
 * - 回傳的 getHistory 讀 ref，不觸發重繪，而且身分穩定。
 */
export function useScreenHistory(
  sessionId: string | null | undefined,
  aiCliRunning: boolean,
  getIdleMs: (() => number) | undefined,
) {
  const historyRef = useRef<string[]>([]);
  const wasBusyRef = useRef(false);
  const getIdleMsRef = useRef(getIdleMs);
  useEffect(() => { getIdleMsRef.current = getIdleMs; }, [getIdleMs]);

  // 工具結束或換了分頁／session：舊歷史不屬於新的情境，清掉。
  useEffect(() => {
    historyRef.current = [];
    wasBusyRef.current = false;
  }, [sessionId, aiCliRunning]);

  useEffect(() => {
    if (!aiCliRunning || !sessionId) return;
    const id = setInterval(() => {
      const fn = getIdleMsRef.current;
      const busy = fn ? fn() < IDLE_MS : false;
      if (busy) { wasBusyRef.current = true; return; }
      if (!wasBusyRef.current) return;
      wasBusyRef.current = false;
      const screen = serializeTerminal(sessionId);
      if (screen) historyRef.current = pushScreen(historyRef.current, screen);
    }, POLL_MS);
    return () => clearInterval(id);
  }, [aiCliRunning, sessionId]);

  const getHistory = useCallback(() => historyRef.current, []);
  return { getHistory };
}
