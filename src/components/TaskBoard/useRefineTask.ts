import { useCallback, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";
import { invokeAiChat } from "../../ipc/ai";
import { buildRefinePrompt, parseRefined, type RefinedTask } from "./refinePrompts";

/**
 * 把草擬的工作內容交給 AI 改寫成可執行的任務指令。
 *
 * 只負責「打一次 AI 並解析回覆」——要不要套用、怎麼還原，都是呼叫端
 * （TaskEditorDialog）的事，因為那裡才知道使用者當下的輸入框狀態。
 */
export function useRefineTask() {
  const { t } = useLocale();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refine = useCallback(
    async (
      body: string,
      projectDir: string,
      needTitle: boolean,
      providerId?: string,
    ): Promise<RefinedTask | null> => {
      setBusy(true);
      setError(null);
      try {
        const reply = await invokeAiChat(
          [{ role: "user", content: buildRefinePrompt(body, projectDir, needTitle) }],
          "task-refine",
          providerId,
        );
        const parsed = parseRefined(reply.content ?? "");
        if (!parsed) {
          setError(t.board_refine_err_empty);
          return null;
        }
        return parsed;
      } catch (e) {
        setError(String(e));
        return null;
      } finally {
        setBusy(false);
      }
    },
    [t],
  );

  return { refine, busy, error };
}
