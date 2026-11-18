import { confirm } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef } from "react";

import { useLocale } from "../contexts/LocaleContext";
import { canvasConfirmDecide, onCanvasConfirmDispatch } from "../ipc/canvas";
import { unlistenOnCleanup } from "../lib/eventSubscription";
import { restoreWebviewFocus } from "../lib/webviewFocus";

/**
 * Ask AI 要派工時跳出確認框（App 最上層常駐，任何分頁都看得到）。
 * 按「派工」才建卡；取消、關閉或錯誤一律當作拒絕。5 分鐘沒回應後端自己會取消。
 */
export function useCanvasDispatchConfirm() {
  const { t } = useLocale();
  const tRef = useRef(t);
  useEffect(() => {
    tRef.current = t;
  }, [t]);

  useEffect(() => {
    const un = onCanvasConfirmDispatch((req) => {
      void (async () => {
        let ok = false;
        try {
          const s = tRef.current;
          ok = await confirm(s.canvas_confirm_body(req.project, req.agent, req.title, req.instruction), {
            title: s.canvas_confirm_title,
            kind: "warning",
            okLabel: s.canvas_confirm_ok,
            cancelLabel: s.canvas_confirm_cancel,
          });
        } catch {
          ok = false;
        }
        await restoreWebviewFocus();
        await canvasConfirmDecide(req.id, ok).catch(() => false);
      })();
    });
    return unlistenOnCleanup(un, "canvas://confirm-dispatch");
  }, []);
}
