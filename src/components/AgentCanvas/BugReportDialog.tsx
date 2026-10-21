import { useEffect, useMemo, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { assignAgent, type AgentRow } from "../../ipc/agents";
import { addAttachment, createTask, type TaskWithAttachments } from "../../ipc/tasks";
import { bugCard, pickAssignee } from "./bugReport";
import { localizedCardTitle } from "./cardTitle";

/**
 * 回報問題：人工測試時發現的錯誤，描述＋截圖（貼上、拖進來或選檔）一次送出並直接派給工程師，
 * 不經過模組庫；之後照常審核、合併、驗收。
 */
export function BugReportDialog({
  projectId,
  agents,
  tasks,
  moduleId,
  onClose,
  onSent,
}: {
  projectId: string;
  agents: AgentRow[];
  tasks: TaskWithAttachments[];
  /** 從某個模組按「回報問題」時帶入。 */
  moduleId?: string | null;
  onClose: () => void;
  onSent: () => void;
}) {
  const { t } = useLocale();
  const [text, setText] = useState("");
  const [images, setImages] = useState<{ file: File; url: string | null }[]>([]);
  const modules = useMemo(
    () => tasks.filter((x) => x.card_kind === "work" && x.status === "done" && x.agent_id && !x.archived_at),
    [tasks],
  );
  const [related, setRelated] = useState<string>(moduleId ?? "");
  const coders = agents.filter((a) => a.kind === "coder" && !a.produces_spec);
  const relatedOwner = modules.find((m) => m.id === related)?.agent_id ?? null;
  // 有選相關模組時預設交給原本做的人（他的分支上就是那段程式）；使用者改過就照使用者的。
  const [assignee, setAssignee] = useState<string | null>(null);
  const effective = assignee ?? (relatedOwner && coders.some((a) => a.id === relatedOwner) ? relatedOwner : "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const shot = useRef(0);

  // 關掉對話框時釋放縮圖的網址。
  const imagesRef = useRef(images);
  useEffect(() => {
    imagesRef.current = images;
  }, [images]);
  useEffect(
    () => () => {
      for (const i of imagesRef.current) if (i.url) URL.revokeObjectURL(i.url);
    },
    [],
  );

  const addFiles = (files: FileList | File[] | null | undefined) => {
    const list = Array.from(files ?? []).filter((f) => f.type.startsWith("image/"));
    if (list.length === 0) return false;
    setImages((cur) => [
      ...cur,
      ...list.map((f) => {
        // 貼上的截圖都叫 image.png：重新命名，附件才不會撞名。
        shot.current += 1;
        const name = /^image\.\w+$/i.test(f.name) || !f.name ? `screenshot-${shot.current}.${f.type.split("/")[1] || "png"}` : f.name;
        const file = name === f.name ? f : new File([f], name, { type: f.type });
        return { file, url: typeof URL.createObjectURL === "function" ? URL.createObjectURL(file) : null };
      }),
    ]);
    return true;
  };

  const submit = async () => {
    const target = pickAssignee(agents, tasks, effective || null);
    if (!text.trim() || !target) return;
    setBusy(true);
    setError(null);
    try {
      const mod = modules.find((m) => m.id === related);
      const { title, body } = bugCard(text, images.length > 0 ? t.ac_bug_instructions : t.ac_bug_instructions_noshot, mod ? t.ac_bug_related(localizedCardTitle(mod, t)) : null);
      const id = await createTask(projectId, {
        title,
        body,
        project_dir: target.project_dir,
        parallel_ok: true,
        interactive: false,
        use_bridge: false,
        bridge_tiers: null,
        label: t.ac_bug_label,
        isolate_worktree: null,
      });
      for (const { file } of images) {
        const bytes = new Uint8Array(await (file.arrayBuffer ? file.arrayBuffer() : new Response(file).arrayBuffer()));
        await addAttachment(projectId, id, file.name, bytes);
      }
      await assignAgent(projectId, id, target.id);
      onSent();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const autoName = pickAssignee(agents, tasks, null)?.name;

  return (
    <div className="task-dialog-backdrop">
      <div
        className="task-dialog ac-bug-dialog"
        role="dialog"
        aria-label={t.ac_bug_title}
        onPaste={(e) => {
          if (addFiles(e.clipboardData?.files)) e.preventDefault();
        }}
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => {
          e.preventDefault();
          addFiles(e.dataTransfer?.files);
        }}
      >
        <h3 className="task-dialog-title">{t.ac_bug_title}</h3>
        <p className="ac-dialog-desc">{t.ac_bug_desc}</p>
        <label className="task-field">
          <span className="task-field-label">{t.ac_bug_what}</span>
          <textarea
            className="task-field-input ac-bug-text"
            autoFocus
            value={text}
            placeholder={t.ac_bug_placeholder}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) void submit();
            }}
          />
        </label>
        <div className="task-field">
          <span className="task-field-label">{t.ac_bug_shots}</span>
          <div className="ac-bug-shots" data-testid="bug-shots">
            {images.map((img, i) => (
              <span key={`${img.file.name}-${i}`} className="ac-bug-shot" title={img.file.name}>
                {img.url ? <img src={img.url} alt={img.file.name} /> : <span>{img.file.name}</span>}
                <button
                  type="button"
                  aria-label={t.ac_bug_remove_shot(img.file.name)}
                  onClick={() => {
                    if (img.url) URL.revokeObjectURL(img.url);
                    setImages((cur) => cur.filter((_, j) => j !== i));
                  }}
                >
                  ×
                </button>
              </span>
            ))}
            <button type="button" className="ac-bug-add" onClick={() => fileRef.current?.click()}>
              {t.ac_bug_shots_hint}
            </button>
            <input
              ref={fileRef}
              type="file"
              accept="image/*"
              multiple
              hidden
              data-testid="bug-file"
              onChange={(e) => {
                addFiles(e.target.files);
                e.target.value = "";
              }}
            />
          </div>
        </div>
        <div className="ac-bug-row">
          <label className="task-field">
            <span className="task-field-label">{t.ac_bug_module}</span>
            <select className="task-field-input" value={related} onChange={(e) => setRelated(e.target.value)}>
              <option value="">{t.ac_bug_module_none}</option>
              {modules.map((m) => (
                <option key={m.id} value={m.id}>
                  {localizedCardTitle(m, t)}
                </option>
              ))}
            </select>
          </label>
          <label className="task-field">
            <span className="task-field-label">{t.ac_bug_assignee}</span>
            <select className="task-field-input" value={effective} onChange={(e) => setAssignee(e.target.value)}>
              <option value="">{t.ac_bug_auto(autoName ?? "—")}</option>
              {coders.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.name}
                  {a.role ? ` · ${a.role}` : ""}
                </option>
              ))}
            </select>
          </label>
        </div>
        {coders.length === 0 && <p className="ac-bad-text">{t.ac_bug_no_coder}</p>}
        {error && <p className="ac-bad-text">{error}</p>}
        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--ghost" onClick={onClose}>
            {t.ac_verify_close}
          </button>
          <button
            className="aiterm-btn aiterm-btn--primary"
            disabled={!text.trim() || coders.length === 0 || busy}
            title={t.ac_bug_send_tip}
            onClick={() => void submit()}
          >
            {t.ac_bug_send}
          </button>
        </div>
      </div>
    </div>
  );
}
