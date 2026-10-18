import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { invokeAiComplete } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { getProjectBrief, setProjectBrief } from "../../ipc/tasks";
import { ModelPickerButton } from "../ModelPickerButton";
import { BRIEF_REFINE_SYSTEM, buildBriefRefineMessage, parseBriefReply } from "./briefRefine";

/** 超過就提醒：每張卡都會帶著這段（後端放進提示時另有更寬的硬上限）。 */
const BRIEF_SOFT_LIMIT = 2000;
/** 跟工作卡的「AI 潤飾」共用模型偏好。 */
const REFINE_PROVIDER_KEY = "aiterm_refine_provider";

/** 專案簡介：每張 Agent 卡派工時放在提示前面，連同自動產生的團隊分工。 */
export function ProjectBriefDialog({
  projectId,
  onSaved,
  onClose,
}: {
  projectId: string;
  onSaved: (brief: string) => void;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const [text, setText] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState("");
  const [refining, setRefining] = useState(false);
  /** 潤飾前的內容；有值才顯示「還原」。 */
  const [beforeRefine, setBeforeRefine] = useState<string | null>(null);

  // 沿用工作卡潤飾的慣例：記住上次選的 → is_default → 清單第一個。
  useEffect(() => {
    let alive = true;
    listProviders()
      .then((list) => {
        if (!alive) return;
        setProviders(list);
        const remembered = localStorage.getItem(REFINE_PROVIDER_KEY);
        const fallback = list.find((p) => p.is_default)?.id ?? list[0]?.id ?? "";
        setProviderId(remembered && list.some((p) => p.id === remembered) ? remembered : fallback);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const refine = async () => {
    const draft = (text ?? "").trim();
    if (!draft) return;
    setRefining(true);
    setError(null);
    try {
      const reply = await invokeAiComplete(
        [{ role: "user", content: buildBriefRefineMessage(draft) }],
        BRIEF_REFINE_SYSTEM,
        "brief-refine",
        providerId || undefined,
      );
      const refined = parseBriefReply(reply.content ?? "");
      if (!refined) {
        setError(t.board_refine_err_empty);
        return;
      }
      if (providerId) localStorage.setItem(REFINE_PROVIDER_KEY, providerId);
      setBeforeRefine(text);
      setText(refined);
    } catch (e) {
      setError(String(e));
    } finally {
      setRefining(false);
    }
  };

  useEffect(() => {
    let alive = true;
    getProjectBrief(projectId)
      .then((b) => alive && setText(b ?? ""))
      .catch((e) => {
        if (!alive) return;
        setText("");
        setError(String(e));
      });
    return () => {
      alive = false;
    };
  }, [projectId]);

  const save = async () => {
    if (text === null) return;
    setBusy(true);
    setError(null);
    try {
      await setProjectBrief(projectId, text.trim() || null);
      onSaved(text.trim());
      onClose();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  const count = [...(text ?? "")].length;
  return (
    <div className="task-dialog-backdrop">
      <div className="task-dialog ac-brief" role="dialog" aria-label={t.ac_brief_title}>
        <h3 className="task-dialog-title">{t.ac_brief_title}</h3>
        <p className="ac-dialog-desc">{t.ac_brief_desc}</p>
        <div className="ac-brief-tools">
          <ModelPickerButton providers={providers} selectedId={providerId} onChange={setProviderId} disabled={refining} />
          <button
            type="button"
            className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
            data-testid="brief-refine"
            title={t.ac_brief_refine_hint}
            disabled={refining || busy || !(text ?? "").trim()}
            onClick={() => void refine()}
          >
            {refining ? t.board_refine_busy : t.board_refine}
          </button>
          {beforeRefine !== null && (
            <button
              type="button"
              className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
              data-testid="brief-refine-undo"
              disabled={refining}
              onClick={() => {
                setText(beforeRefine);
                setBeforeRefine(null);
              }}
            >
              {t.board_refine_undo}
            </button>
          )}
        </div>
        <textarea
          className="task-field-input ac-brief-text"
          aria-label={t.ac_brief_title}
          value={text ?? ""}
          disabled={text === null || refining}
          placeholder={t.ac_brief_placeholder}
          onChange={(e) => setText(e.target.value)}
        />
        <div className={`ac-brief-count${count > BRIEF_SOFT_LIMIT ? " ac-bad-text" : ""}`}>{t.ac_brief_count(count)}</div>
        {count > BRIEF_SOFT_LIMIT && <p className="task-field-hint ac-bad-text">{t.ac_brief_long}</p>}
        {error && <p className="ac-bad-text">{error}</p>}
        <div className="task-dialog-actions">
          <button className="aiterm-btn aiterm-btn--secondary" onClick={onClose}>
            {t.ac_brief_cancel}
          </button>
          <button className="aiterm-btn aiterm-btn--primary" disabled={busy || refining || text === null} onClick={() => void save()}>
            {t.ac_brief_save}
          </button>
        </div>
      </div>
    </div>
  );
}
