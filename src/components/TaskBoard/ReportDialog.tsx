import { useCallback, useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";

import { useLocale } from "../../contexts/LocaleContext";
import { ArtifactHtmlFrame } from "../ArtifactPanel/ArtifactHtmlFrame";
import { ModelPickerButton } from "../ModelPickerButton";
import { writeTextFile } from "../../ipc/fs";
import { listProviders, type ProviderInfo } from "../../ipc/provider";
import { listReports, readReport, type ReportInfo } from "../../ipc/reports";
import { listTasks, type TaskWithAttachments } from "../../ipc/tasks";
import { useWorkReport } from "./useWorkReport";
import type { ReportStyle } from "./reportPrompts";

/** 記住上次選的模型，key 沿用這個 repo「記住偏好」的慣例。 */
const PROVIDER_KEY = "aiterm_report_provider";

/**
 * 工作報告視窗：先選風格 → 產生 → 呈現，側邊可切換這個專案的歷史報告。
 *
 * HTML 用既有的 `ArtifactHtmlFrame` 渲染——那是個 sandbox iframe，
 * 刻意不給 `allow-same-origin`，所以報告裡的 script 碰不到主視窗，
 * 更碰不到 Tauri 的 IPC。不要為了任何理由改那個設定。
 */
/**
 * 歷史清單上的產生時間。
 *
 * 同一個專案的報告標題幾乎都一樣（AI 傾向拿專案名當標題），所以只列
 * 標題根本分不出哪份是哪份——時間才是實際用來辨識的資訊。
 *
 * `saved_at` 是後端給的 Unix 秒（檔案的修改時間）。用 `toLocaleString`
 * 而不是自己格式化：使用者的日期慣例交給平台決定就好。
 */
function formatSavedAt(savedAt: number): string {
  if (!savedAt) return "";
  return new Date(savedAt * 1000).toLocaleString(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/**
 * 「這次會包含什麼」的摘要。
 *
 * 除了讓風格選擇頁不要空一大片，它主要回答使用者在按下去之前真正想知道
 * 的事：**這次要跑多久**。第一階段是每張「已完成但還沒有摘要」的卡片各
 * 打一次 AI，那個張數就是耗時的主因；全部都有快取時只剩第二階段一次呼叫，
 * 會很快——這件事不講的話，使用者會以為每次都要等一樣久。
 */
function ReportScope({
  cards,
  t,
}: {
  cards: TaskWithAttachments[];
  t: ReturnType<typeof useLocale>["t"];
}) {
  if (cards.length === 0) {
    return (
      <div className="report-scope" data-testid="report-scope">
        {t.report_scope_empty}
      </div>
    );
  }

  const byStatus = (s: string) => cards.filter((c) => c.status === s).length;
  const pending = cards.filter((c) => c.status === "done" && !c.ai_summary).length;

  return (
    <div className="report-scope" data-testid="report-scope">
      <div className="report-scope-title">
        {t.report_scope_title} <strong>{cards.length}</strong> {t.report_scope_total}
      </div>
      <div className="report-scope-cols">
        {(
          [
            ["planning", t.report_col_planning],
            ["queued", t.report_col_queued],
            ["running", t.report_col_running],
            ["done", t.report_col_done],
          ] as const
        ).map(([status, label]) => (
          <div key={status} className="report-scope-col">
            <span className="report-scope-col-count">{byStatus(status)}</span>
            <span className="report-scope-col-label">{label}</span>
          </div>
        ))}
      </div>
      {pending > 0 ? (
        <div className="report-scope-pending" data-testid="report-scope-pending">
          <strong>{pending}</strong> {t.report_scope_pending}
        </div>
      ) : (
        <div className="report-scope-cached">{t.report_scope_cached}</div>
      )}
    </div>
  );
}

export function ReportDialog({
  projectId,
  projectName,
  onClose,
}: {
  projectId: string;
  projectName: string;
  onClose: () => void;
}) {
  const { t } = useLocale();
  const { generate, cancel, busy, progress, html, error, rawReply, setHtml } = useWorkReport(
    projectId,
    projectName,
  );
  const [history, setHistory] = useState<ReportInfo[]>([]);
  const [picked, setPicked] = useState<string | null>(null);
  const [started, setStarted] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  /** 這個專案的卡片，只用來算「這次會包含什麼」的統計。 */
  const [cards, setCards] = useState<TaskWithAttachments[] | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [selectedProviderId, setSelectedProviderId] = useState("");

  const refreshHistory = useCallback(async () => {
    setHistory(await listReports(projectId));
  }, [projectId]);

  // 初次掛載抓一次歷史清單。之後歷史只在「產生完成」時需要重抓，
  // 由 start() 在 generate() 完成後明確呼叫 refreshHistory()——不用
  // effect 盯著 html 變化，避免點歷史報告 (setHtml) 誤觸發多餘的重抓。
  //
  // 這裡刻意仿照 TranscriptDialog.tsx 的 `alive` guard 寫法而不是
  // `void refreshHistory()`：後者會被 react-hooks/set-state-in-effect
  // 判定為「effect 唯一的內容就是呼叫一個以 setState 結尾的函式」而報錯，
  // 帶 cleanup 的 inline fetch 寫法不會。
  useEffect(() => {
    let alive = true;
    void listReports(projectId).then((rows) => {
      if (alive) setHistory(rows);
    });
    return () => {
      alive = false;
    };
  }, [projectId]);

  // 載入可選的模型清單，並決定預設選中哪一個：上次記住的 → is_default →
  // 清單第一個。記住的 id 若已經不在清單裡（provider 被刪掉了），不能卡住
  // 選不到任何東西，一樣要退回預設。
  useEffect(() => {
    let alive = true;
    void listProviders().then((list) => {
      if (!alive) return;
      setProviders(list);
      const remembered = localStorage.getItem(PROVIDER_KEY);
      const fallback = list.find((p) => p.is_default)?.id ?? list[0]?.id ?? "";
      setSelectedProviderId(
        remembered && list.some((p) => p.id === remembered) ? remembered : fallback,
      );
    });
    return () => {
      alive = false;
    };
  }, []);

  // 卡片統計：讓使用者在按下去之前知道這次會包含什麼、大概要跑多久。
  // 「需要重新整理」的張數就是第一階段的 AI 呼叫次數，那是耗時的主因。
  useEffect(() => {
    let alive = true;
    void listTasks(projectId)
      .then((rows) => {
        if (alive) setCards(rows);
      })
      .catch(() => {
        // 統計只是輔助資訊，抓不到就不顯示，不該擋住產生報告這件事。
        if (alive) setCards([]);
      });
    return () => {
      alive = false;
    };
  }, [projectId]);

  const start = (style: ReportStyle) => {
    setStarted(true);
    setPicked(null);
    setCancelling(false);
    if (selectedProviderId) localStorage.setItem(PROVIDER_KEY, selectedProviderId);
    // `generate` 的回傳型別是 Promise<void>，但測試裡的 mock 版本是
    // `vi.fn()`（回傳 undefined）——包一層 Promise.resolve 讓兩邊都安全。
    void Promise.resolve(generate(style, selectedProviderId || undefined)).then(refreshHistory);
  };

  const onCancel = () => {
    // cancel() 不會中斷正在飛的那次 AI 呼叫（invokeAiChat 沒有 abort
    // 機制），所以取消後最久還要等一次呼叫回來。要讓使用者知道這件事，
    // 不然按了沒反應會以為壞了。
    setCancelling(true);
    cancel();
  };

  const openHistory = async (info: ReportInfo) => {
    setPicked(info.filename);
    setHtml(await readReport(projectId, info.filename));
  };

  const saveAs = async () => {
    if (!html) return;
    const path = await save({
      defaultPath: `${projectName}-report.html`,
      filters: [{ name: "HTML", extensions: ["html"] }],
    });
    if (typeof path === "string") await writeTextFile(path, html);
  };

  const progressText = progress
    ? t.report_progress
        .replace("{done}", String(progress.done))
        .replace("{total}", String(progress.total))
    : t.report_generating;

  return (
    <div className="task-dialog-backdrop" onClick={onClose}>
      <div className="report-dialog" onClick={(e) => e.stopPropagation()}>
        <div className="report-head">
          <h3>{t.report_title} — {projectName}</h3>
          <div className="report-head-actions">
            {html && (
              <button className="tb-btn tb-btn--ghost" onClick={() => void saveAs()}>
                {t.report_save_as}
              </button>
            )}
            <button className="tb-btn tb-btn--ghost" onClick={onClose}>{t.report_close}</button>
          </div>
        </div>

        <div className="report-body">
          <aside className="report-history">
            <div className="report-history-title">{t.report_history}</div>
            {history.length === 0 ? (
              <div className="report-history-empty" data-testid="report-history-empty">
                {t.report_history_empty}
              </div>
            ) : (
              history.map((h) => (
                <button
                  key={h.filename}
                  className={`report-history-item${picked === h.filename ? " report-history-item--active" : ""}`}
                  onClick={() => void openHistory(h)}
                >
                  <span className="report-history-item-title">{h.title ?? h.filename}</span>
                  <span
                    className="report-history-item-time"
                    data-testid={`report-history-time-${h.filename}`}
                  >
                    {formatSavedAt(h.saved_at)}
                  </span>
                </button>
              ))
            )}
          </aside>

          <main className="report-main">
            {!started && !html && (
              <>
                <div className="report-model-row">
                  <span className="report-model-row-label">{t.report_model}</span>
                  <ModelPickerButton
                    providers={providers}
                    selectedId={selectedProviderId}
                    onChange={setSelectedProviderId}
                  />
                </div>
                <div className="report-style-picker">
                  <button
                    className="report-style"
                    data-testid="report-style-review"
                    onClick={() => start("review")}
                  >
                    <strong>{t.report_style_review}</strong>
                    <span>{t.report_style_review_hint}</span>
                  </button>
                  <button
                    className="report-style"
                    data-testid="report-style-formal"
                    onClick={() => start("formal")}
                  >
                    <strong>{t.report_style_formal}</strong>
                    <span>{t.report_style_formal_hint}</span>
                  </button>
                </div>
                {cards && <ReportScope cards={cards} t={t} />}
              </>
            )}

            {busy && (
              <div className="report-progress">
                <span data-testid="report-progress">
                  {cancelling ? t.report_cancelling : progressText}
                </span>
                {!cancelling && (
                  <button
                    className="tb-btn tb-btn--ghost"
                    data-testid="report-cancel"
                    onClick={onCancel}
                  >
                    {t.report_cancel}
                  </button>
                )}
              </div>
            )}

            {error && <div className="report-error">{error}</div>}
            {rawReply && (
              <details className="report-raw" open>
                <summary>{t.report_raw_reply}</summary>
                <pre>{rawReply}</pre>
              </details>
            )}

            {html && <ArtifactHtmlFrame html={html} title={t.report_title} />}
          </main>
        </div>
      </div>
    </div>
  );
}
