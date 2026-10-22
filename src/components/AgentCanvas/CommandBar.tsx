import { useEffect, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { invokeAiComplete } from "../../ipc/ai";
import { askAgent, assignAgent, createAgent, getReferences, type AgentRow } from "../../ipc/agents";
import { listProviders } from "../../ipc/provider";
import { addAttachment, createTask, getProjectBrief, setProjectBrief, type TaskWithAttachments } from "../../ipc/tasks";
import { MarkdownText } from "../../lib/markdown";
import { ModelPickerButton } from "../ModelPickerButton";
import type { ProviderInfo } from "../../ipc/provider";
import { dispatchInput, INPUT_KINDS, routeInput, type AddAttachment, type DispatchResult, type InputKind, type RouteDecision } from "./routeInput";
import type { StartDeps } from "./startFromRequirement";

/** 跟專案簡介、需求先行共用模型偏好。 */
const REFINE_PROVIDER_KEY = "aiterm_refine_provider";

const defaultDeps: StartDeps = {
  getBrief: getProjectBrief,
  setBrief: setProjectBrief,
  getReferences,
  complete: async (messages, system, streamId, providerId) => (await invokeAiComplete(messages, system, streamId, providerId)).content ?? "",
  createAgent,
  createTask,
  assignAgent,
};

type Phase =
  | { kind: "idle" }
  | { kind: "routing" }
  | { kind: "plan"; decision: RouteDecision; usedDefault: boolean }
  | { kind: "working" }
  /** 問題交給 Agent 了，等那張問答卡完成。 */
  | { kind: "asking"; taskId: string; agentName: string }
  | { kind: "answer"; text: string };

/**
 * 常駐輸入框：什麼都往裡面丟。AI 先判斷是新需求、小修改、bug 還是問題，
 * 顯示「我打算這樣處理」讓使用者確認（或改類型），確認後才建卡派工、或直接回答。
 */
export function CommandBar({
  projectId,
  repo,
  agents,
  tasks,
  onDispatched,
  deps = defaultDeps,
  attach = addAttachment,
  ask = askAgent,
}: {
  projectId: string;
  /** 專案記住的程式庫；null = 沒有（沒有就不能建卡）。 */
  repo: string | null;
  agents: AgentRow[];
  tasks: TaskWithAttachments[];
  /** 建好卡（或回答完）之後：外面要重新整理、顯示結果。 */
  onDispatched: (r: DispatchResult) => void;
  deps?: StartDeps;
  attach?: AddAttachment;
  ask?: NonNullable<Parameters<typeof dispatchInput>[3]["askAgent"]>;
}) {
  const { t } = useLocale();
  const [text, setText] = useState("");
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState<string | undefined>(undefined);
  /** 附的圖：可有可無。 */
  const [images, setImages] = useState<{ file: File; url: string | null }[]>([]);
  const fileRef = useRef<HTMLInputElement>(null);
  const shot = useRef(0);
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
  /** 供應商清單載入完成時的預設選擇：剛打開就送出時，要等它而不是誤判成「沒有 AI」。 */
  const defaultProvider = useRef<Promise<string | undefined>>(Promise.resolve(undefined));

  useEffect(() => {
    let alive = true;
    defaultProvider.current = listProviders()
      .then((list) => {
        const remembered = localStorage.getItem(REFINE_PROVIDER_KEY);
        const pick = remembered && list.some((p) => p.id === remembered) ? remembered : (list.find((p) => p.is_default)?.id ?? list[0]?.id);
        if (alive) {
          setProviders(list);
          setProviderId(pick);
        }
        return pick;
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  const busy = phase.kind === "routing" || phase.kind === "working";

  // 等 Agent 回答：問答卡完成就顯示它寫的答案；失敗就說明原因。
  const asked = phase.kind === "asking" ? tasks.find((x) => x.id === phase.taskId) : undefined;
  useEffect(() => {
    if (phase.kind !== "asking" || !asked || asked.status !== "done") return;
    if (asked.outcome === "success" && asked.ai_summary?.trim()) {
      setPhase({ kind: "answer", text: asked.ai_summary.trim() });
    } else {
      setError(`${t.ac_cmd_ask_failed}${asked.error_message ? `：${asked.error_message}` : ""}`);
      setPhase({ kind: "idle" });
    }
  }, [phase, asked, t]);
  const kindLabel = (k: InputKind) => t[`ac_cmd_kind_${k}` as const];
  const kindTip = (k: InputKind) => t[`ac_cmd_kind_${k}_tip` as const];

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

  const clearImages = () => {
    for (const i of imagesRef.current) if (i.url) URL.revokeObjectURL(i.url);
    setImages([]);
  };

  /** 只貼圖沒打字也能送：圖就是需求，卡片標題與分類用一句固定的話。 */
  const content = text.trim() || (images.length > 0 ? t.ac_cmd_image_only : "");

  const analyze = async () => {
    if (!content || busy) return;
    setError(null);
    setNotice(null);
    setPhase({ kind: "routing" });
    const pid = providerId ?? (await defaultProvider.current);
    const { decision, usedDefault } = await routeInput(deps, projectId, content, pid);
    setPhase({ kind: "plan", decision: usedDefault && !pid ? { ...decision, reason: t.ac_cmd_no_ai_reason } : decision, usedDefault });
  };

  const execute = async (kind: InputKind) => {
    if (!repo) return;
    setPhase({ kind: "working" });
    setError(null);
    try {
      const pid = providerId ?? (await defaultProvider.current);
      const r = await dispatchInput(
        kind,
        content,
        { projectId, repoDir: repo, agents, tasks, t, providerId: pid },
        { ...deps, addAttachment: attach, askAgent: ask },
        images.map((i) => i.file),
      );
      if (r.kind === "question") {
        setPhase({ kind: "asking", taskId: r.taskId ?? "", agentName: r.agentName ?? "" });
      } else {
        setPhase({ kind: "idle" });
        setNotice(r.fellBack ? t.ac_cmd_fell_back(r.agentName ?? "") : t.ac_cmd_done(kindLabel(r.kind), r.agentName ?? ""));
      }
      setText("");
      clearImages();
      onDispatched(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setPhase({ kind: "idle" });
    }
  };

  return (
    <div
      className="ac-cmd"
      data-testid="ac-cmd"
      onPaste={(e) => {
        if (addFiles(e.clipboardData?.files)) e.preventDefault();
      }}
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => {
        e.preventDefault();
        addFiles(e.dataTransfer?.files);
      }}
    >
      {phase.kind === "plan" && (
        <div className="ac-cmd-plan" data-testid="ac-cmd-plan" role="group" aria-label={t.ac_cmd_plan_title}>
          <b className="ac-cmd-plan-title">{t.ac_cmd_plan_title}</b>
          <div className="ac-cmd-kinds">
            {INPUT_KINDS.map((k) => (
              <button
                key={k}
                type="button"
                className={`ac-cmd-kind${phase.decision.kind === k ? " ac-cmd-kind--on" : ""}`}
                aria-pressed={phase.decision.kind === k}
                title={kindTip(k)}
                data-testid={`ac-cmd-kind-${k}`}
                onClick={() => setPhase({ ...phase, decision: { ...phase.decision, kind: k } })}
              >
                {kindLabel(k)}
              </button>
            ))}
          </div>
          {phase.decision.reason && <p className="ac-cmd-reason">{phase.decision.reason}</p>}
          <p className="ac-cmd-tip">{kindTip(phase.decision.kind)}</p>
          {phase.decision.kind === "question" && images.length > 0 && (
            <p className="ac-bad-text ac-cmd-note" data-testid="ac-cmd-question-no-image">
              {t.ac_cmd_question_no_image}
            </p>
          )}
          <div className="ac-cmd-actions">
            <button type="button" className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => setPhase({ kind: "idle" })}>
              {t.ac_cmd_cancel}
            </button>
            <button
              type="button"
              className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
              data-testid="ac-cmd-confirm"
              disabled={!repo || (phase.decision.kind === "question" && images.length > 0)}
              onClick={() => void execute(phase.decision.kind)}
            >
              {t.ac_cmd_confirm}
            </button>
          </div>
        </div>
      )}
      {phase.kind === "asking" && (
        <div className="ac-cmd-answer" data-testid="ac-cmd-asking" role="status">
          <p>{t.ac_cmd_asking(phase.agentName)}</p>
          <button type="button" className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => setPhase({ kind: "idle" })}>
            {t.ac_cmd_close}
          </button>
        </div>
      )}
      {phase.kind === "answer" && (
        <div className="ac-cmd-answer ac-cmd-answer--long" data-testid="ac-cmd-answer" role="status">
          <div className="ac-cmd-answer-head">
            <b>{t.ac_cmd_answer_title}</b>
            <button type="button" className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" onClick={() => setPhase({ kind: "idle" })}>
              {t.ac_cmd_close}
            </button>
          </div>
          <div className="ac-cmd-answer-body">
            <MarkdownText text={phase.text} />
          </div>
        </div>
      )}
      {notice && phase.kind === "idle" && (
        <p className="ac-good-text ac-cmd-note" data-testid="ac-cmd-notice" role="status">
          {notice}
        </p>
      )}
      {error && (
        <p className="ac-bad-text ac-cmd-note" data-testid="ac-cmd-error" role="alert">
          {error}
        </p>
      )}
      {images.length > 0 && (
        <div className="ac-cmd-shots" data-testid="ac-cmd-shots">
          {images.map((img, i) => (
            <span key={`${img.file.name}-${i}`} className="ac-bug-shot" title={img.file.name}>
              {img.url ? <img src={img.url} alt={img.file.name} /> : <span>{img.file.name}</span>}
              <button
                type="button"
                aria-label={t.ac_cmd_remove_shot(img.file.name)}
                disabled={busy}
                onClick={() => {
                  if (img.url) URL.revokeObjectURL(img.url);
                  setImages((cur) => cur.filter((_, j) => j !== i));
                }}
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}
      <div className="ac-cmd-row">
        <button
          type="button"
          className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
          data-testid="ac-cmd-attach"
          title={t.ac_cmd_attach_tip}
          disabled={busy}
          onClick={() => fileRef.current?.click()}
        >
          {t.ac_cmd_attach}
        </button>
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          multiple
          hidden
          data-testid="ac-cmd-file"
          onChange={(e) => {
            addFiles(e.target.files);
            e.target.value = "";
          }}
        />
        <textarea
          className="ac-cmd-input"
          data-testid="ac-cmd-input"
          rows={1}
          value={text}
          placeholder={t.ac_cmd_placeholder}
          disabled={busy}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            // 輸入法組字中的 Enter 是選字，不是送出。
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void analyze();
            }
          }}
        />
        {providers.length > 0 && (
          <span className="ac-cmd-model">
            <ModelPickerButton providers={providers} selectedId={providerId ?? ""} onChange={setProviderId} disabled={busy} />
          </span>
        )}
        <button
          type="button"
          className="aiterm-btn aiterm-btn--primary aiterm-btn--sm"
          data-testid="ac-cmd-send"
          disabled={busy || !content || phase.kind === "plan"}
          onClick={() => void analyze()}
        >
          {phase.kind === "routing" ? t.ac_cmd_routing : phase.kind === "working" ? t.ac_cmd_working : "↵"}
        </button>
      </div>
    </div>
  );
}
