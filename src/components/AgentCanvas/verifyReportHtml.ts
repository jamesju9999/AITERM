import type { VerifyItem, VerifyReportView } from "../../ipc/agents";

/** 匯出報告用到的字串（由呼叫端依語系傳入）。 */
export interface VerifyReportLabels {
  title: string;
  pass: string;
  fail: string;
  manual: string;
  steps: string;
  expected: string;
  actual: string;
  notes: string;
  evidence: string;
  overall: string;
  summary: (pass: number, fail: number, manual: number) => string;
}

const ORDER: Record<VerifyItem["status"], number> = { fail: 0, manual: 1, pass: 2 };
const ICON: Record<VerifyItem["status"], string> = { pass: "✓", fail: "✗", manual: "?" };

/** 沒通過的在前，再來是要你確認的，最後是通過的。 */
export const sortItems = (items: VerifyItem[]): VerifyItem[] => [...items].sort((a, b) => ORDER[a.status] - ORDER[b.status]);

const esc = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");

/**
 * 驗收報告匯出成一個獨立的 HTML 檔：截圖直接內嵌（data URL），任何瀏覽器都能開、
 * 也能列印成 PDF。所有來自報告的文字都經過跳脫——內容是 Agent 寫的，不能信任。
 */
export function verifyReportHtml(view: VerifyReportView, meta: { project: string; date: string }, l: VerifyReportLabels): string {
  const report = view.report;
  const items = report ? sortItems(report.items) : [];
  const count = (s: VerifyItem["status"]) => items.filter((i) => i.status === s).length;
  const label = { pass: l.pass, fail: l.fail, manual: l.manual };
  const field = (name: string, value?: string | null) =>
    value ? `<div class="f"><b>${esc(name)}</b><span>${esc(value)}</span></div>` : "";
  const body = items
    .map((i) => {
      const texts = i.evidence.filter((e) => !view.images[e]);
      const shots = i.evidence
        .filter((e) => view.images[e])
        .map((e) => `<figure><img src="${esc(view.images[e])}" alt="${esc(e)}"><figcaption>${esc(e)}</figcaption></figure>`)
        .join("");
      return `<section class="item ${i.status}">
<h2><span class="s" title="${esc(label[i.status])}">${ICON[i.status]}</span>${esc(i.criterion)}</h2>
${field(l.steps, i.steps)}${field(l.expected, i.expected)}${field(l.actual, i.actual)}${field(l.notes, i.notes)}${field(l.evidence, texts.join("\n"))}
${shots ? `<div class="shots">${shots}</div>` : ""}
</section>`;
    })
    .join("\n");
  return `<!doctype html>
<html lang="zh-Hant"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(`${l.title} · ${meta.project}`)}</title>
<style>
body { font: 14px/1.6 -apple-system, "Segoe UI", "Noto Sans TC", sans-serif; color: #1f2328; max-width: 900px; margin: 32px auto; padding: 0 16px; }
h1 { font-size: 22px; margin: 0 0 4px; }
.meta { color: #59636e; margin: 0 0 20px; }
.item { border: 1px solid #d1d9e0; border-radius: 8px; padding: 12px 16px; margin: 0 0 12px; break-inside: avoid; }
.item.fail { border-color: #cf222e; }
.item.manual { border-style: dashed; }
h2 { font-size: 15px; margin: 0 0 6px; display: flex; gap: 8px; }
.s { width: 1.2em; text-align: center; }
.pass .s { color: #1a7f37; } .fail .s { color: #cf222e; } .manual .s { color: #9a6700; }
.f { display: grid; grid-template-columns: 5em 1fr; gap: 8px; color: #31373d; }
.f b { color: #59636e; font-weight: 500; }
.f span { white-space: pre-wrap; word-break: break-word; }
.shots { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 8px; }
figure { margin: 0; } figure img { max-width: 420px; width: 100%; border: 1px solid #d1d9e0; border-radius: 6px; }
figcaption { font-size: 12px; color: #59636e; }
.overall { background: #f6f8fa; border-radius: 8px; padding: 12px 16px; white-space: pre-wrap; }
</style></head><body>
<h1>${esc(l.title)}</h1>
<p class="meta">${esc(meta.project)} · ${esc(meta.date)} · ${esc(l.summary(count("pass"), count("fail"), count("manual")))}${report?.project_type ? ` · ${esc(report.project_type)}` : ""}</p>
${body}
${report?.notes ? `<h2>${esc(l.overall)}</h2><div class="overall">${esc(report.notes)}</div>` : ""}
</body></html>
`;
}
