import type { TaskOutcome, TaskStatus, TaskWithAttachments } from "../../ipc/tasks";

/** 報告風格。`review` 給自己看，`formal` 給上司／客戶看。 */
export type ReportStyle = "review" | "formal";

/**
 * 第二階段最多帶幾張卡片。超過就取最近的——這是防止極端情況下輸入
 * 爆掉的保險，不是預期會踩到的線。
 */
export const MAX_CARDS = 100;

/** 一張卡片摘要的字數上限，寫進提示詞裡要求模型遵守。 */
const SUMMARY_MAX_CHARS = 300;

const STATUS_LABEL: Record<TaskStatus, string> = {
  planning: "規劃中",
  queued: "待執行",
  running: "執行中",
  done: "已完成",
};

const OUTCOME_LABEL: Record<TaskOutcome, string> = {
  success: "成功",
  failed: "失敗",
  cancelled: "已取消",
};

/**
 * 第一階段：把一張已完成卡片的對話記錄摘要成一段短文字。
 *
 * `transcript` 為 null 代表記錄檔不存在（被刪掉、或那次執行沒留下）。
 * 這不是錯誤——照樣用欄位資料產生摘要，只是內容會比較粗略。
 */
export function buildSummaryPrompt(
  card: TaskWithAttachments,
  transcript: string | null,
): string {
  const lines = [
    "請把下面這個工作項目的執行過程，整理成一段給工作報告用的摘要。",
    "",
    `標題：${card.title}`,
    `工作內容：${card.body || "（未填寫）"}`,
    `工作目錄：${card.project_dir}`,
    `結果：${card.outcome ? OUTCOME_LABEL[card.outcome] : "未知"}`,
  ];
  if (card.error_message) lines.push(`錯誤訊息：${card.error_message}`);
  lines.push("");
  if (transcript) {
    lines.push("以下是這次執行的終端機對話記錄：", "", transcript);
  } else {
    lines.push("（這次執行沒有對話記錄，請只根據上面的欄位資料整理。）");
  }
  lines.push(
    "",
    `請用繁體中文寫一段 ${SUMMARY_MAX_CHARS} 字以內的摘要，說明實際做了什麼、`,
    "過程中遇到什麼問題、最後結果如何。只輸出摘要本文，不要加標題或前言。",
  );
  return lines.join("\n");
}

function cardLine(card: TaskWithAttachments): string {
  const status = STATUS_LABEL[card.status];
  const outcome = card.outcome ? `／${OUTCOME_LABEL[card.outcome]}` : "";
  const parts = [`- 【${status}${outcome}】${card.title}`];
  if (card.body) parts.push(`  內容：${card.body}`);
  parts.push(`  工作目錄：${card.project_dir}`);
  if (card.error_message) parts.push(`  錯誤：${card.error_message}`);
  if (card.ai_summary) parts.push(`  執行摘要：${card.ai_summary}`);
  return parts.join("\n");
}

const STYLE_INSTRUCTIONS: Record<ReportStyle, string> = {
  review: [
    "這份報告是給我自己回顧進度用的。重點放在：",
    "- 目前做到哪個階段",
    "- 哪些工作卡住了、為什麼",
    "- 接下來應該優先處理什麼",
    "可以直接講技術細節，也請直接點出失敗的原因，不需要修飾。",
  ].join("\n"),
  formal: [
    "這份報告是要給主管或客戶看的正式工作報告。重點放在：",
    "- 這段期間完成了哪些工作",
    "- 各項工作的具體成果",
    "語氣正式、精簡，少講技術細節，重點在產出而不是過程。",
    "尚未完成的工作簡短帶過即可，不要強調失敗與錯誤訊息。",
  ].join("\n"),
};

/**
 * 第二階段：把全部卡片（含第一階段的摘要）合成一份 HTML 報告。
 *
 * 卡片超過 `MAX_CARDS` 時只取最近的，**並且在提示詞裡講明**——不講的話
 * 模型會把「最近 100 張」當成專案的全部，報告的結論會失真。
 */
export function buildReportPrompt(
  cards: TaskWithAttachments[],
  style: ReportStyle,
  projectName: string,
): string {
  const truncated = cards.length > MAX_CARDS;
  const used = truncated ? cards.slice(-MAX_CARDS) : cards;

  const lines = [
    `請為專案「${projectName}」整理一份工作報告。`,
    "",
    STYLE_INSTRUCTIONS[style],
    "",
  ];
  if (truncated) {
    lines.push(
      `注意：這個專案共有 ${cards.length} 個工作項目，因為篇幅限制，下面只列出最近的 ${MAX_CARDS} 個。`,
      "請在報告中說明這件事，不要把這些當成專案的全部。",
      "",
    );
  }
  lines.push("工作項目如下：", "", ...used.map(cardLine), "");
  lines.push(
    "請把報告寫成一份完整的 HTML 文件，放在 ```artifact-html 區塊裡。",
    "文件要有 <title>（會成為報告的標題）、清楚的段落結構、以及適當的內嵌 <style>。",
    "內容用繁體中文。",
  );
  return lines.join("\n");
}
