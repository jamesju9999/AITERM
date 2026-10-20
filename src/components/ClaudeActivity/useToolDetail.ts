import { useEffect, useState } from "react";
import { getClaudeToolDetail, type ToolDetail } from "../../ipc/claudeActivity";

/**
 * 取選中工具節點的明細。`version` 變了（例如工具從執行中變成完成）就重取，
 * 因為輸出是結束後才有的。結果連同它對應的 key 一起存，key 對不上就當作還沒有，
 * 所以換節點時不會閃出上一個節點的內容，也不必在 effect 裡同步清空。
 */
export function useToolDetail(sessionId: string | undefined, toolId: string | null, version: string): ToolDetail | null {
  const key = `${sessionId ?? ""}|${toolId ?? ""}|${version}`;
  const [state, setState] = useState<{ key: string; detail: ToolDetail | null }>({ key: "", detail: null });

  useEffect(() => {
    if (!sessionId || !toolId) return;
    let alive = true;
    getClaudeToolDetail(sessionId, toolId)
      .then((detail) => {
        if (alive) setState({ key, detail });
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [key, sessionId, toolId]);

  return state.key === key ? state.detail : null;
}
