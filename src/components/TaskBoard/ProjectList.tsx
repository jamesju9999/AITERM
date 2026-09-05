import { useCallback, useEffect, useState } from "react";
import { confirm, open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import {
  listProjects,
  openProject,
  removeProject,
  type ProjectInfo,
} from "../../ipc/projects";
import { onTasksUpdated } from "../../ipc/tasks";
import { ProjectCreateDialog } from "./ProjectCreateDialog";

export function ProjectList({ onOpen }: { onOpen: (projectId: string) => void }) {
  const { t } = useLocale();
  const [projects, setProjects] = useState<ProjectInfo[]>([]);
  const [creating, setCreating] = useState(false);

  const refresh = useCallback(async () => {
    setProjects(await listProjects());
  }, []);

  useEffect(() => {
    void refresh();
    const un = onTasksUpdated(() => void refresh());
    return () => void un.then((f) => f());
  }, [refresh]);

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
    if (!(await confirm(t.proj_remove_confirm))) return;
    const deleteFolder = await confirm(t.proj_remove_folder_confirm);
    await removeProject(p.id, deleteFolder);
    await refresh();
  };

  const pickExisting = async () => {
    const picked = await open({ filters: [{ name: "AITerm 專案", extensions: ["aitprj"] }] });
    if (typeof picked !== "string") return;
    await openProject(picked);
    await refresh();
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
            const total = p.counts.planning + p.counts.queued + p.counts.running + p.counts.done;
            const broken = p.status !== "ok";
            return (
              <div
                key={p.id}
                className={`project-card${broken ? " project-card--broken" : ""}`}
                data-testid={`project-card-${p.id}`}
              >
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
                    <div className="project-card-meta">
                      <span data-testid={`project-total-${p.id}`}>
                        {total} {t.proj_tasks_count}
                      </span>
                      {p.counts.running > 0 && (
                        <span
                          className="project-card-running"
                          data-testid={`project-running-${p.id}`}
                        >
                          ● {p.counts.running} {t.proj_running}
                        </span>
                      )}
                    </div>
                  )}
                </button>
                <button
                  className="tb-btn tb-btn--danger-ghost"
                  data-testid={`project-remove-${p.id}`}
                  onClick={() => void remove(p)}
                >
                  {t.proj_remove}
                </button>
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
            void refresh();
            onOpen(id);
          }}
        />
      )}
    </div>
  );
}
