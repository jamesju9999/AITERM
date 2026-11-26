import { useEffect, useState } from "react";

import { checkRepo, type AgentRow } from "../../ipc/agents";

/** 有 Agent 在用、但這台電腦上不存在的程式庫路徑（專案資料夾複製過來時會發生）。 */
export function useMissingRepoDirs(agents: AgentRow[]): string[] {
  const [missing, setMissing] = useState<string[]>([]);
  const key = [...new Set(agents.map((a) => a.project_dir))].sort().join("\n");
  useEffect(() => {
    let alive = true;
    const dirs = key ? key.split("\n") : [];
    void Promise.all(dirs.map((d) => checkRepo(d).then((r) => (r.status === "missing" ? d : null)).catch(() => null))).then((r) => {
      if (alive) setMissing(r.filter((d): d is string => d !== null));
    });
    return () => {
      alive = false;
    };
  }, [key]);
  return missing;
}
