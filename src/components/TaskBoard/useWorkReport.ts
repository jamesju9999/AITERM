import { useCallback, useRef, useState } from "react";

import { useLocale } from "../../contexts/LocaleContext";

import { invokeAiChat, type AiError } from "../../ipc/ai";
import { saveReport } from "../../ipc/reports";
import {
  listTasks,
  readTranscript,
  setSummary,
  type TaskWithAttachments,
} from "../../ipc/tasks";
import { splitArtifactFence } from "../../lib/artifactFence";
import { buildReportPrompt, buildSummaryPrompt, type ReportStyle } from "./reportPrompts";

/**
 * 這個錯誤是不是「AI 還沒設定」。
 *
 * Tauri 可能把 `AiError` 直接交過來，也可能包在 `Error` 裡（message 是那串
 * JSON），兩種都要吃。
 *
 * 刻意寫成本地小函式而不 import `agentLoop.ts` 匯出的 `normalizeAiError`：
 * 那個模組是終端機 agent 迴圈，值層級還連著 `formatAiError`、`ipc/web` 與
 * heredoc 修補，為了一個布林判斷把整條迴圈拉進報告流程並不划算。這裡只需要
 * 判斷一種 kind。
 */
function isNotConfigured(err: unknown): boolean {
  if (err && typeof err === "object" && "kind" in err) {
    return (err as AiError).kind === "not_configured";
  }
  if (err instanceof Error) {
    try {
      const parsed: unknown = JSON.parse(err.message);
      return !!parsed && typeof parsed === "object" && "kind" in parsed
        && (parsed as AiError).kind === "not_configured";
    } catch {
      return false;
    }
  }
  return false;
}

export interface ReportProgress {
  /** 已經處理完的卡片數。 */
  done: number;
  /** 這一輪要處理的卡片總數（只算需要補摘要的）。 */
  total: number;
}

/**
 * 工作報告的產生流程。
 *
 * 兩階段：先把每張「已完成且還沒有摘要」的卡片各自摘要並寫回快取，
 * 再把全部卡片合成一份 HTML 報告。已完成的卡片不可變，所以摘要是永久
 * 快取——第二次產報告時只有新完成的卡片需要重跑。
 */
export function useWorkReport(projectId: string, projectName: string) {
  // useLocale 在沒有 provider 時會自己從 localStorage 兜底，所以這裡
  // 直接用是安全的（見 LocaleContext.tsx）。錯誤訊息走 i18n，不然
  // en 語系的使用者會看到寫死的中文。
  const { t } = useLocale();
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<ReportProgress | null>(null);
  const [html, setHtml] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  /** artifact 抽不出來時保留原始回覆給使用者看，不要默默失敗。 */
  const [rawReply, setRawReply] = useState<string | null>(null);
  const cancelled = useRef(false);

  const cancel = useCallback(() => {
    cancelled.current = true;
  }, []);

  const generate = useCallback(
    async (style: ReportStyle, providerId?: string) => {
      setBusy(true);
      setError(null);
      setHtml(null);
      setRawReply(null);
      cancelled.current = false;

      try {
        const cards = await listTasks(projectId);
        if (cards.length === 0) {
          setError(t.report_err_no_cards);
          return;
        }

        // ── 第一階段：補上缺少的摘要 ──
        const needSummary = cards.filter((c) => c.status === "done" && !c.ai_summary);
        setProgress({ done: 0, total: needSummary.length });

        const summaries = new Map<string, string>();
        for (const [i, c] of needSummary.entries()) {
          if (cancelled.current) return;
          try {
            // 後端對「沒有 transcript_path」的卡片回空字串而不是報錯，所以不必
            // 先檢查欄位。只有記錄檔真的讀不到才會 reject——那當成「沒有記錄」，
            // 不是錯誤，摘要照樣用欄位資料產生。
            const transcript = await readTranscript(projectId, c.id).catch(() => null);
            const reply = await invokeAiChat(
              [{ role: "user", content: buildSummaryPrompt(c, transcript || null) }],
              `report-summary-${c.id}`,
              providerId,
            );
            const text = (reply.content ?? "").trim();
            if (text) {
              await setSummary(projectId, c.id, text);
              summaries.set(c.id, text);
            }
          } catch (e) {
            // AI 根本沒設定的話，後面每一張都會失敗——直接中止比讓
            // 使用者等一輪無意義的重試有意義。
            if (isNotConfigured(e)) {
              setError(t.report_err_not_configured);
              return;
            }
            // 其他錯誤：略過這張，繼續下一張。一張失敗不該讓整份報告白跑。
          }
          setProgress({ done: i + 1, total: needSummary.length });
        }
        if (cancelled.current) return;

        // ── 第二階段：合成 ──
        const enriched = cards.map((c): TaskWithAttachments => {
          const fresh = summaries.get(c.id);
          return fresh ? { ...c, ai_summary: fresh } : c;
        });
        const reply = await invokeAiChat(
          [{ role: "user", content: buildReportPrompt(enriched, style, projectName) }],
          `report-${projectId}`,
          providerId,
          false,
          "zh-TW",
          true, // supportsArtifacts — 後端才會接上 artifact 協定的說明
        );
        if (cancelled.current) return;

        const text = reply.content ?? "";
        const { artifact } = splitArtifactFence(text);
        if (!artifact || artifact.kind !== "html") {
          setRawReply(text);
          setError(t.report_err_no_artifact);
          return;
        }

        setHtml(artifact.content);
        try {
          await saveReport(projectId, artifact.content);
        } catch (e) {
          // 報告本身已經產生了，存檔失敗不該讓它消失——顯示出來並說明。
          setError(`${t.report_err_save_failed}${String(e)}`);
        }
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
        setProgress(null);
      }
    },
    [projectId, projectName, t],
  );

  return { generate, cancel, busy, progress, html, error, rawReply, setHtml };
}
