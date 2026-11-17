import { useEffect, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { invokeAiComplete } from "../../ipc/ai";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { ModelPickerButton } from "../ModelPickerButton";
import { buildGoalRefineMessage, GOAL_REFINE_SYSTEM, parseGoalReply } from "./goalRefine";

/** 跟工作卡、專案簡介、定時器的「AI 潤飾」共用模型偏好。 */
const REFINE_PROVIDER_KEY = "aiterm_refine_provider";

interface Props {
  systemName: string;
  goal: string;
  onGoal: (g: string) => void;
  /** 潤飾中，呼叫端要把輸入框鎖起來。 */
  onBusy?: (busy: boolean) => void;
  disabled?: boolean;
  onError: (msg: string | null) => void;
}

/** 目標欄下方的「✨ AI 潤飾」列：選模型、潤飾、還原。定時任務與「請 AI 操作」共用。 */
export function GoalRefineBar({ systemName, goal, onGoal, onBusy, disabled, onError }: Props) {
  const { t } = useLocale();
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [providerId, setProviderId] = useState("");
  const [refining, setRefining] = useState(false);
  /** 潤飾前的內容；有值才顯示「還原」。 */
  const [before, setBefore] = useState<string | null>(null);

  // 沿用其他「AI 潤飾」的慣例：記住上次選的 → is_default → 清單第一個。
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

  const busy = (b: boolean) => {
    setRefining(b);
    onBusy?.(b);
  };

  const refine = async () => {
    const draft = goal.trim();
    if (!draft) return;
    busy(true);
    onError(null);
    try {
      const reply = await invokeAiComplete(
        [{ role: "user", content: buildGoalRefineMessage(draft, systemName) }],
        GOAL_REFINE_SYSTEM,
        "system-job-goal-refine",
        providerId || undefined,
      );
      const refined = parseGoalReply(reply.content ?? "");
      if (!refined) {
        onError(t.board_refine_err_empty);
        return;
      }
      if (providerId) localStorage.setItem(REFINE_PROVIDER_KEY, providerId);
      setBefore(goal);
      onGoal(refined);
    } catch (e) {
      onError(typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e));
    } finally {
      busy(false);
    }
  };

  return (
    <div className="systems__refinebar">
      <ModelPickerButton providers={providers} selectedId={providerId} onChange={setProviderId} disabled={refining || disabled} />
      <button type="button" className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm" data-testid="goal-refine" disabled={refining || disabled || !goal.trim()} onClick={() => void refine()}>
        {refining ? t.board_refine_busy : t.board_refine}
      </button>
      {before !== null && (
        <button
          type="button"
          className="aiterm-btn aiterm-btn--ghost aiterm-btn--sm"
          data-testid="goal-refine-undo"
          disabled={refining}
          onClick={() => {
            onGoal(before);
            setBefore(null);
          }}
        >
          {t.board_refine_undo}
        </button>
      )}
    </div>
  );
}
