import { useCallback, useEffect, useRef, useState } from "react";
import { isAiCliCommand } from "../../lib/aiCliCommand";

export interface SidebarBlock {
  id: string;
  command: string;
  status: "running" | "completed" | "failed";
}

/**
 * 建議側欄的開關狀態，以及「終端機裡現在有沒有 AI 命令列工具在跑」。
 *
 * - `aiCliRunning` 純粹由最後一個 block 推導，沒有額外狀態可以過期。
 * - **不會在偵測到工具時自動開啟**：側欄一開，終端機欄就變窄、觸發 PTY resize，
 *   而那正好是 TUI 剛啟動、最怕被縮放打斷的時刻。偵測只用來點亮開關按鈕。
 * - 使用者「在工具執行中」打開的，工具結束時自動收起（那個側欄是為它開的）；
 *   使用者在沒有工具時手動打開的，由使用者自己關。
 */
export function useSuggestionSidebar(blocks: readonly SidebarBlock[], customNames: readonly string[]) {
  const latest = blocks[blocks.length - 1];
  const aiCliRunning = !!latest && latest.status === "running" && isAiCliCommand(latest.command, customNames);

  const [open, setOpen] = useState(false);
  const autoCloseRef = useRef(false);
  const aiCliRunningRef = useRef(aiCliRunning);
  useEffect(() => { aiCliRunningRef.current = aiCliRunning; }, [aiCliRunning]);

  useEffect(() => {
    if (open && autoCloseRef.current && !aiCliRunning) {
      autoCloseRef.current = false;
      setOpen(false);
    }
  }, [open, aiCliRunning]);

  const toggle = useCallback(() => {
    setOpen((wasOpen) => {
      if (!wasOpen) autoCloseRef.current = aiCliRunningRef.current;
      return !wasOpen;
    });
  }, []);
  const close = useCallback(() => setOpen(false), []);

  return { aiCliRunning, open, toggle, close };
}
