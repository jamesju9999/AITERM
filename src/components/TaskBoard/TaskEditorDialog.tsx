import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { ModelPickerButton } from "../ModelPickerButton";
import { usedDirs } from "../../ipc/projects";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import {
  addAttachment,
  createTask,
  removeAttachment,
  updateTask,
  type AttachmentRow,
  type TaskWithAttachments,
} from "../../ipc/tasks";
import { useRefineTask } from "./useRefineTask";

const LAST_DIR_KEY = "aiterm_last_task_dir";
/** 潤飾用的模型偏好，跟工作報告分開記——兩者的合適模型不一定一樣。 */
const REFINE_PROVIDER_KEY = "aiterm_refine_provider";

export function TaskEditorDialog({
  projectId,
  card,
  onClose,
  onSaved,
}: {
  projectId: string;
  card: TaskWithAttachments | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t } = useLocale();
  const isEdit = card !== null;
  const [title, setTitle] = useState(card?.title ?? "");
  const [body, setBody] = useState(card?.body ?? "");
  const [dir, setDir] = useState(card?.project_dir ?? localStorage.getItem(LAST_DIR_KEY) ?? "");
  const [parallelOk, setParallelOk] = useState(card?.parallel_ok ?? true);
  const [interactive, setInteractive] = useState(card?.interactive ?? false);
  // Edit mode: already-uploaded rows, hanging off the existing card id.
  const [attachments, setAttachments] = useState<AttachmentRow[]>(card?.attachments ?? []);
  // Create mode: a brand-new card has no id yet, so picked files can't be
  // uploaded until `createTask` returns one — buffered here and uploaded in
  // `save()` right after that id comes back.
  const [pendingFiles, setPendingFiles] = useState<File[]>([]);
  const [busy, setBusy] = useState(false);

  // 專案不綁資料夾（工作可散布在多個 repo），所以列出這個專案已經
  // 用過的目錄讓使用者一鍵選取，不必每次重新瀏覽。
  const [dirChoices, setDirChoices] = useState<string[]>([]);
  useEffect(() => {
    let alive = true;
    void usedDirs(projectId).then((dirs) => {
      if (alive) setDirChoices(dirs);
    });
    return () => {
      alive = false;
    };
  }, [projectId]);

  // 潤飾用的模型：沿用 ReportDialog 的慣例（記住上次選的 → is_default →
  // 清單第一個），記住的 id 已經不在清單裡時要退回預設，不能卡在選不到。
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  useEffect(() => {
    let alive = true;
    void listProviders().then((list) => {
      if (!alive) return;
      setProviders(list);
      const remembered = localStorage.getItem(REFINE_PROVIDER_KEY);
      const fallback = list.find((p) => p.is_default)?.id ?? list[0]?.id ?? "";
      setSelectedProviderId(
        remembered && list.some((p) => p.id === remembered) ? remembered : fallback,
      );
    });
    return () => {
      alive = false;
    };
  }, []);

  const { refine, busy: refining, error: refineError } = useRefineTask();
  /**
   * 潤飾前的標題與內容。有值就代表「有東西可以還原」，還原鈕的顯示條件
   * 也是它。標題一起記是因為潤飾可能順便填了標題——只還原內容會留下一個
   * 不是使用者自己寫的標題。
   */
  const [beforeRefine, setBeforeRefine] = useState<{ title: string; body: string } | null>(null);

  const runRefine = async () => {
    if (!body.trim()) return;
    const snapshot = { title, body };
    // 標題已經有字就不要求 AI 產生——那是使用者明確寫下的意圖，不能蓋掉。
    const result = await refine(body, dir, !title.trim(), selectedProviderId || undefined);
    if (!result) return; // 失敗時輸入框原封不動，錯誤訊息由 hook 提供
    if (selectedProviderId) localStorage.setItem(REFINE_PROVIDER_KEY, selectedProviderId);
    setBeforeRefine(snapshot);
    setBody(result.body);
    if (result.title && !snapshot.title.trim()) setTitle(result.title);
  };

  const undoRefine = () => {
    if (!beforeRefine) return;
    setTitle(beforeRefine.title);
    setBody(beforeRefine.body);
    setBeforeRefine(null);
  };

  const pickDir = async () => {
    const picked = await open({ directory: true, defaultPath: dir || undefined });
    if (typeof picked === "string") {
      setDir(picked);
    }
  };

  const onFiles = async (files: FileList | null) => {
    if (!files) return;
    if (isEdit) {
      for (const f of Array.from(files)) {
        const bytes = new Uint8Array(await f.arrayBuffer());
        const row = await addAttachment(projectId, card.id, f.name, bytes);
        setAttachments((a) => [...a, row]);
      }
    } else {
      setPendingFiles((prev) => [...prev, ...Array.from(files)]);
    }
  };

  const save = async () => {
    setBusy(true);
    try {
      if (dir) localStorage.setItem(LAST_DIR_KEY, dir);
      if (isEdit) {
        await updateTask(projectId, {
          id: card.id,
          title,
          body,
          project_dir: dir,
          parallel_ok: parallelOk,
          interactive,
        });
      } else {
        const newId = await createTask(projectId, {
          title,
          body,
          project_dir: dir,
          parallel_ok: parallelOk,
          interactive,
        });
        for (const f of pendingFiles) {
          const bytes = new Uint8Array(await f.arrayBuffer());
          await addAttachment(projectId, newId, f.name, bytes);
        }
      }
      onSaved();
    } finally {
      setBusy(false);
    }
  };

  const attachmentNames = isEdit
    ? attachments.map((a) => ({ key: a.id, name: a.filename }))
    : pendingFiles.map((f, i) => ({ key: `${i}-${f.name}`, name: f.name }));

  const removeAttachmentAt = (key: string) => {
    if (isEdit) {
      void removeAttachment(projectId, key).then(() =>
        setAttachments((list) => list.filter((x) => x.id !== key)),
      );
    } else {
      setPendingFiles((prev) => prev.filter((f, i) => `${i}-${f.name}` !== key));
    }
  };

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div className="task-dialog" onClick={(e) => e.stopPropagation()}>
        <h3 className="task-dialog-title">{isEdit ? t.board_edit_card : t.board_new_card}</h3>

        <label className="task-field">
          <span className="task-field-label">{t.board_card_title}</span>
          <input
            className="task-field-input"
            data-testid="task-title-input"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
          />
        </label>

        {/* 這一格用 div 而不是 label：裡面有模型選單跟兩個按鈕，包在 label
            裡的話點它們會連帶把焦點搶到 textarea，下拉選單會當場關掉。
            改用 htmlFor 綁定，可及性一樣，行為才對。 */}
        <div className="task-field">
          <div className="task-field-labelrow">
            <label className="task-field-label" htmlFor="task-body-input">
              {t.board_card_body}
            </label>
            <div className="task-refine-actions">
              <ModelPickerButton
                providers={providers}
                selectedId={selectedProviderId}
                onChange={setSelectedProviderId}
              />
              <button
                type="button"
                className="tb-btn tb-btn--ghost tb-btn--tiny"
                data-testid="task-refine"
                title={t.board_refine_hint}
                disabled={refining || !body.trim()}
                onClick={() => void runRefine()}
              >
                {refining ? t.board_refine_busy : t.board_refine}
              </button>
              {beforeRefine && (
                <button
                  type="button"
                  className="tb-btn tb-btn--ghost tb-btn--tiny"
                  data-testid="task-refine-undo"
                  disabled={refining}
                  onClick={undoRefine}
                >
                  {t.board_refine_undo}
                </button>
              )}
            </div>
          </div>
          <textarea
            id="task-body-input"
            className="task-field-input task-field-textarea"
            data-testid="task-body-input"
            rows={5}
            value={body}
            onChange={(e) => setBody(e.target.value)}
          />
          {refineError && (
            <p className="task-field-error" data-testid="task-refine-error">
              {refineError}
            </p>
          )}
        </div>

        <label className="task-field">
          <span className="task-field-label">{t.board_card_folder}</span>
          <div className="task-field-row">
            <input
              className="task-field-input"
              data-testid="task-dir-input"
              value={dir}
              onChange={(e) => setDir(e.target.value)}
            />
            <button type="button" className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm" onClick={() => void pickDir()}>
              {t.board_card_folder_pick}
            </button>
          </div>
          {dirChoices.length > 0 && (
            <div className="task-used-dirs" data-testid="used-dirs-row">
              {dirChoices.map((d) => (
                <button
                  key={d}
                  type="button"
                  className="tb-btn tb-btn--ghost tb-btn--tiny"
                  data-testid={`used-dir-${d}`}
                  onClick={() => setDir(d)}
                >
                  📁 {d}
                </button>
              ))}
            </div>
          )}
        </label>

        <label className="task-field task-field--checkbox">
          <input
            type="checkbox"
            className="task-checkbox"
            checked={interactive}
            onChange={(e) => setInteractive(e.target.checked)}
          />
          <span>{t.board_card_interactive}</span>
        </label>
        <p className="task-field-hint">{t.board_card_interactive_hint}</p>

        {!interactive && (
          <>
            <label className="task-field task-field--checkbox">
              <input
                type="checkbox"
                className="task-checkbox"
                checked={parallelOk}
                onChange={(e) => setParallelOk(e.target.checked)}
              />
              <span>{t.board_card_parallel}</span>
            </label>
            <p className="task-field-hint">{t.board_card_solo_hint}</p>
          </>
        )}

        <div className="task-field">
          <span className="task-field-label">{t.board_card_attachments}</span>
          <p className="task-field-hint">{t.board_card_attachments_hint}</p>
          {attachmentNames.length > 0 && (
            <ul className="task-attachment-list">
              {attachmentNames.map((a) => (
                <li key={a.key} className="task-attachment-row">
                  <span className="task-attachment-name">{a.name}</span>
                  <button
                    type="button"
                    className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
                    aria-label={`${t.board_delete} ${a.name}`}
                    onClick={() => removeAttachmentAt(a.key)}
                  >
                    ✕
                  </button>
                </li>
              ))}
            </ul>
          )}
          <label className="aiterm-btn aiterm-btn--secondary aiterm-btn--sm task-attachment-add">
            + {t.board_add_attachment}
            <input
              type="file"
              multiple
              className="task-attachment-file-input"
              aria-label={t.board_add_attachment}
              onChange={(e) => void onFiles(e.target.files)}
            />
          </label>
        </div>

        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" disabled={busy} onClick={onClose}>
            {t.board_cancel}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary"
            disabled={busy || !title.trim() || !dir.trim()}
            onClick={() => void save()}
          >
            {t.board_save}
          </button>
        </div>
      </div>
    </div>
  );
}
