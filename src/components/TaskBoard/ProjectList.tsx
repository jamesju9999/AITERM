import { useState } from "react";
import { confirm, message, open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { openProject, removeProject, type ProjectInfo } from "../../ipc/projects";
import { ProjectCreateDialog } from "./ProjectCreateDialog";

/**
 * 純呈現：專案清單與它的重新載入都由 TaskBoardView 擁有（分頁列也要用同
 * 一份），這裡不再自己抓一次、也不再自己訂閱 tasks-updated——那會是同一
 * 份資料的第二個來源，兩邊還可能不同步。
 */
export function ProjectList({
  projects,
  onRefresh,
  onOpen,
  onReport,
}: {
  projects: ProjectInfo[];
  onRefresh: () => Promise<void>;
  onOpen: (projectId: string) => void;
  onReport: (projectId: string) => void;
}) {
  const { t } = useLocale();
  const [creating, setCreating] = useState(false);

  const statusLabel = (p: ProjectInfo) =>
    ({
      missing: t.proj_status_missing,
      invalid: t.proj_status_invalid,
      incompatible: t.proj_status_incompatible,
      ok: "",
    })[p.status];

  // 兩段式原生對話框，與 TaskCard 的刪除流程同一套模式。
  // 必須用 @tauri-apps/plugin-dialog 的非同步 confirm，不可用
  // window.confirm——Tauri 的 webview 沒有真正實作它（見 TaskCard.tsx:33 的註解）。
  const remove = async (p: ProjectInfo) => {
    // 後端一定會拒絕移除還有工作在跑的專案，所以在問任何問題之前就擋下來。
    // 不擋的話使用者會被一路帶到「要連同磁碟上的資料夾一起刪除嗎？此動作
    // 無法復原」——整個流程最危險的一問，卻問在一個注定失敗的操作上。
    if (p.counts.running > 0) {
      await message(t.proj_remove_running_blocked, { title: p.name, kind: "warning" });
      return;
    }
    if (!(await confirm(t.proj_remove_confirm))) return;
    const deleteFolder = await confirm(t.proj_remove_folder_confirm);
    try {
      await removeProject(p.id, deleteFolder);
    } catch (e) {
      // counts 只是上一次 projects_list 的快照，可能在按下去之前就過期
      // （工作剛開始跑）。後端的拒絕要講出來，不能靜靜失敗——那會讓
      // 使用者以為自己按錯了。
      await message(`${t.proj_remove_failed}${String(e)}`, { title: p.name, kind: "error" });
      return;
    }
    await onRefresh();
  };

  const pickExisting = async () => {
    const picked = await open({ filters: [{ name: "AITerm 專案", extensions: ["aitprj"] }] });
    if (typeof picked !== "string") return;
    await openProject(picked);
    await onRefresh();
  };

  return (
    <div className="project-list">
      <div className="project-list-head">
        <h2>{t.proj_title}</h2>
        <div className="project-list-actions">
          <button className="tb-btn tb-btn--primary" onClick={() => setCreating(true)}>
            + {t.proj_new}
          </button>
          <button className="tb-btn tb-btn--ghost" onClick={() => void pickExisting()}>
            {t.proj_open_existing}
          </button>
        </div>
      </div>

      {projects.length === 0 ? (
        <div className="project-empty" data-testid="project-empty-state">
          <div className="project-empty-title">{t.proj_empty_title}</div>
          <div className="project-empty-hint">{t.proj_empty_hint}</div>
        </div>
      ) : (
        <div className="project-grid">
          {projects.map((p) => {
            const broken = p.status !== "ok";
            const cells = [
              ["planning", p.counts.planning, t.report_col_planning],
              ["queued", p.counts.queued, t.report_col_queued],
              ["running", p.counts.running, t.report_col_running],
              ["done", p.counts.done, t.report_col_done],
            ] as const;
            return (
              <div
                key={p.id}
                className={`project-card${broken ? " project-card--broken" : ""}`}
                data-testid={`project-card-${p.id}`}
              >
                {/* 卡片主體是一個大按鈕：整張卡都可以點進去，不只名稱那一行。
                    底下的動作列刻意留在按鈕外面，否則點「移除」會冒泡成
                    「進入專案」——那個回歸有測試釘著。 */}
                <button
                  className="project-card-main"
                  disabled={broken}
                  onClick={() => onOpen(p.id)}
                >
                  <div className="project-card-name">{p.name}</div>
                  {p.description && <div className="project-card-desc">{p.description}</div>}
                  {broken ? (
                    <div className="project-card-error" data-testid={`project-error-${p.id}`}>
                      {p.error ?? statusLabel(p)}
                    </div>
                  ) : (
                    <div className="project-card-counts">
                      {cells.map(([key, n, label]) => (
                        <div
                          key={key}
                          className={`project-count${
                            key === "running" && n > 0 ? " project-count--active" : ""
                          }`}
                          data-testid={`project-count-${p.id}-${key}`}
                        >
                          <span className="project-count-n">{n}</span>
                          <span className="project-count-label">{label}</span>
                        </div>
                      ))}
                    </div>
                  )}
                </button>
                <div className="project-card-actions">
                  <button
                    className="tb-btn tb-btn--ghost tb-btn--tiny"
                    data-testid={`project-report-${p.id}`}
                    disabled={broken}
                    onClick={() => onReport(p.id)}
                  >
                    {t.report_short}
                  </button>
                  {/* 破壞性動作刻意低調：平常是灰的，滑鼠移上去才轉紅。
                      跟「報告」一樣顯眼的話，最該小心的那個反而最搶眼。 */}
                  <button
                    className="project-card-remove"
                    data-testid={`project-remove-${p.id}`}
                    onClick={() => void remove(p)}
                  >
                    {t.proj_remove}
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {creating && (
        <ProjectCreateDialog
          onClose={() => setCreating(false)}
          onCreated={(id) => {
            setCreating(false);
            // 先等清單重新載入再開——新專案的分頁要顯示得先存在於
            // projects 裡（TaskBoardView 是照 projects 過濾分頁列的），
            // 不等的話會閃一下專案總覽才進去。
            void onRefresh().then(() => onOpen(id));
          }}
        />
      )}
    </div>
  );
}
