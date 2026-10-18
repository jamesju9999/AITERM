import { useLocale } from "../../contexts/LocaleContext";
import type { RepoCheck } from "../../ipc/agents";

/** 程式庫檢查結果一行字；還不能用但可以修的，旁邊給一鍵初始化。 */
export function RepoStatus({ repo, busy, onFix }: { repo: RepoCheck; busy: boolean; onFix: () => void }) {
  const { t } = useLocale();
  return (
    <div className={`ac-repo-status ac-repo-status--${repo.status}`} data-testid="repo-status" role="status">
      {repo.status === "ok" && <span>✓ {t.ac_repo_ok(repo.branch)}</span>}
      {repo.status === "missing" && <span>✗ {t.ac_repo_missing}</span>}
      {repo.status === "board_folder" && <span>✗ {t.ac_repo_board_folder}</span>}
      {(repo.status === "not_repo" || repo.status === "no_commits") && (
        <>
          <span>⚠ {repo.status === "not_repo" ? t.ac_repo_not_repo : t.ac_repo_no_commits}</span>
          <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" disabled={busy} onClick={onFix}>
            {busy ? t.ac_repo_initing : repo.status === "not_repo" ? t.ac_repo_init : t.ac_repo_first_commit}
          </button>
        </>
      )}
    </div>
  );
}
