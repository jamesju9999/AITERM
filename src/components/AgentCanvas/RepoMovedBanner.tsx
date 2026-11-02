import { confirm, open } from "@tauri-apps/plugin-dialog";
import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { checkRepo, relocateRepo, type AgentRow } from "../../ipc/agents";

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

/**
 * 專案資料夾搬到別台電腦後：Agent 記的程式庫路徑在這裡不存在，派工會失敗。
 * 讓使用者指到這台電腦上的程式庫（要先 clone 好，並推過 Agent 的分支），一次換掉。
 */
export function RepoMovedBanner({
  projectId,
  agents,
  dirs,
  onDone,
}: {
  projectId: string;
  agents: AgentRow[];
  dirs: string[];
  onDone: () => void;
}) {
  const { t } = useLocale();
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ bad: boolean; text: string } | null>(null);

  const relocate = async (oldDir: string) => {
    setNote(null);
    const picked = await open({ directory: true, multiple: false });
    if (typeof picked !== "string") return;
    setBusy(true);
    try {
      const check = await checkRepo(picked);
      if (check.status !== "ok") {
        setNote({ bad: true, text: t.ac_moved_not_usable(picked) });
        return;
      }
      if (!(await confirm(t.ac_moved_confirm(oldDir, picked), { kind: "warning" }))) return;
      const r = await relocateRepo(projectId, oldDir, picked);
      setNote({ bad: r.missing_branches.length > 0, text: r.missing_branches.length ? t.ac_moved_done_missing(r.agents, r.missing_branches) : t.ac_moved_done(r.agents) });
      onDone();
    } catch (e) {
      setNote({ bad: true, text: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="ac-tools" data-testid="ac-repo-moved" role="alert">
      <div className="ac-tools-head">
        <span>⚠ {t.ac_moved_title}</span>
      </div>
      {dirs.map((d) => (
        <div key={d} className="ac-tools-row">
          <span className="ac-tools-what">{t.ac_moved_row(agents.filter((a) => a.project_dir === d).length)}</span>
          <code className="ac-tools-cmd">{d}</code>
          <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} data-testid="ac-repo-moved-pick" onClick={() => void relocate(d)}>
            {t.ac_moved_pick}
          </button>
        </div>
      ))}
      <span className="ac-tools-note">{t.ac_moved_hint}</span>
      {note && <span className={note.bad ? "ac-bad-text" : "ac-good-text"} data-testid="ac-repo-moved-note">{note.text}</span>}
    </div>
  );
}
