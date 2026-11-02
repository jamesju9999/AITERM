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
  project: string;
  date: string;
  projectType: string;
  /** 「驗收結論」欄位名。 */
  conclusion: string;
  verdictPass: string;
  verdictFail: string;
  overview: string;
  detail: string;
  criterion: string;
  result: string;
  generatedBy: string;
}

const ORDER: Record<VerifyItem["status"], number> = { fail: 0, manual: 1, pass: 2 };
const ICON: Record<VerifyItem["status"], string> = { pass: "✓", fail: "✗", manual: "?" };

/** 沒通過的在前，再來是要你確認的，最後是通過的。 */
export const sortItems = (items: VerifyItem[]): VerifyItem[] => [...items].sort((a, b) => ORDER[a.status] - ORDER[b.status]);

const esc = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");

/**
 * 驗收報告匯出成一個獨立的 HTML 檔：截圖直接內嵌（data URL），任何瀏覽器都能開、
 * 也能列印成 PDF（A4、保留顏色、每個項目不跨頁）。所有來自報告的文字都經過跳脫——內容是 Agent 寫的，不能信任。
 */
export function verifyReportHtml(view: VerifyReportView, meta: { project: string; date: string }, l: VerifyReportLabels): string {
  const report = view.report;
  const items = report ? sortItems(report.items) : [];
  const count = (s: VerifyItem["status"]) => items.filter((i) => i.status === s).length;
  const label = { pass: l.pass, fail: l.fail, manual: l.manual };
  const badge = (st: VerifyItem["status"]) => `<span class="badge ${st}">${ICON[st]} ${esc(label[st])}</span>`;
  const field = (name: string, value?: string | null) =>
    value ? `<div class="f"><b>${esc(name)}</b><span>${esc(value)}</span></div>` : "";
  const total = items.length;
  const verdictPass = report?.verdict === "pass";

  const stat = (st: VerifyItem["status"]) =>
    `<div class="stat ${st}"><div class="n">${count(st)}</div><div class="k">${esc(label[st])}</div></div>`;
  const bar = total
    ? `<div class="bar" role="img" aria-label="${esc(l.summary(count("pass"), count("fail"), count("manual")))}">${(["pass", "fail", "manual"] as const)
        .filter((st) => count(st) > 0)
        .map((st) => `<i class="${st}" style="flex:${count(st)}"></i>`)
        .join("")}</div>`
    : "";
  const overview = items
    .map((i, n) => `<tr class="${i.status}"><td class="no">${n + 1}</td><td><a href="#i${n + 1}">${esc(i.criterion)}</a></td><td class="res">${badge(i.status)}</td></tr>`)
    .join("\n");
  const body = items
    .map((i, n) => {
      const texts = i.evidence.filter((e) => !view.images[e]);
      const shots = i.evidence
        .filter((e) => view.images[e])
        .map((e) => `<figure><img src="${esc(view.images[e])}" alt="${esc(e)}"><figcaption>${esc(e)}</figcaption></figure>`)
        .join("");
      return `<section class="item ${i.status}" id="i${n + 1}">
<h3><span class="no">${n + 1}</span><span class="t">${esc(i.criterion)}</span>${badge(i.status)}</h3>
${field(l.steps, i.steps)}${field(l.expected, i.expected)}${field(l.actual, i.actual)}${field(l.notes, i.notes)}${field(l.evidence, texts.join("\n"))}
${shots ? `<div class="shots">${shots}</div>` : ""}
</section>`;
    })
    .join("\n");
  return `<!doctype html>
<html lang="zh-Hant"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(`${l.title} · ${meta.project}`)}</title>
<style>
@page { size: A4; margin: 16mm 14mm; }
* { box-sizing: border-box; -webkit-print-color-adjust: exact; print-color-adjust: exact; }
body { font: 14px/1.65 -apple-system, "Segoe UI", "Noto Sans TC", "PingFang TC", sans-serif; color: #1f2328; background: #f3f5f8; margin: 0; }
.page { max-width: 880px; margin: 24px auto; background: #fff; padding: 40px 48px; border-radius: 10px; box-shadow: 0 1px 4px rgba(31, 35, 40, .12); }
header { border-bottom: 3px solid #1f3a5f; padding-bottom: 16px; margin-bottom: 20px; }
.kicker { font-size: 12px; letter-spacing: .12em; text-transform: uppercase; color: #59636e; }
h1 { font-size: 28px; line-height: 1.25; margin: 4px 0 12px; color: #1f3a5f; }
.meta { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 8px 24px; margin: 0; }
.meta div { display: flex; flex-direction: column; }
.meta dt { font-size: 12px; color: #59636e; }
.meta dd { margin: 0; font-weight: 600; word-break: break-word; }
.verdict { display: flex; align-items: center; gap: 12px; border-radius: 8px; padding: 12px 18px; margin: 0 0 16px; font-size: 18px; font-weight: 700; }
.verdict small { font-size: 12px; font-weight: 500; opacity: .8; }
.verdict.pass { background: #dafbe1; color: #116329; border: 1px solid #4ac26b; }
.verdict.fail { background: #ffebe9; color: #a40e26; border: 1px solid #ff8182; }
.stats { display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; margin: 0 0 12px; }
.stat { border: 1px solid #d1d9e0; border-radius: 8px; padding: 10px 14px; text-align: center; }
.stat .n { font-size: 26px; font-weight: 700; line-height: 1.2; }
.stat .k { font-size: 12px; color: #59636e; }
.stat.pass .n { color: #1a7f37; } .stat.fail .n { color: #cf222e; } .stat.manual .n { color: #9a6700; }
.bar { display: flex; height: 8px; border-radius: 4px; overflow: hidden; background: #eaeef2; margin: 0 0 24px; }
.bar i.pass { background: #2da44e; } .bar i.fail { background: #cf222e; } .bar i.manual { background: #d4a72c; }
h2 { font-size: 17px; margin: 28px 0 10px; padding-left: 10px; border-left: 4px solid #1f3a5f; color: #1f3a5f; }
table { width: 100%; border-collapse: collapse; font-size: 13px; }
th { text-align: left; background: #eef2f7; color: #31373d; font-weight: 600; }
th, td { padding: 6px 10px; border-bottom: 1px solid #d8dee4; vertical-align: top; }
td.no, th.no { width: 2.5em; text-align: center; color: #59636e; }
td.res, th.res { width: 8em; white-space: nowrap; }
a { color: inherit; text-decoration: none; } a:hover { text-decoration: underline; }
.badge { display: inline-block; padding: 0 9px; border-radius: 999px; font-size: 12px; font-weight: 600; line-height: 20px; white-space: nowrap; }
.badge.pass { background: #dafbe1; color: #116329; } .badge.fail { background: #ffebe9; color: #a40e26; } .badge.manual { background: #fff8c5; color: #7d4e00; }
.item { border: 1px solid #d1d9e0; border-left-width: 4px; border-radius: 6px; padding: 10px 16px 12px; margin: 0 0 12px; break-inside: avoid; }
.item.pass { border-left-color: #2da44e; } .item.fail { border-left-color: #cf222e; } .item.manual { border-left-color: #d4a72c; }
h3 { font-size: 15px; margin: 0 0 8px; display: flex; align-items: baseline; gap: 10px; }
h3 .no { color: #59636e; font-weight: 500; min-width: 1.6em; }
h3 .t { flex: 1; }
.f { display: grid; grid-template-columns: 6em 1fr; gap: 8px; padding: 3px 0; color: #31373d; border-top: 1px dashed #e1e6eb; }
.f b { color: #59636e; font-weight: 500; }
.f span { white-space: pre-wrap; word-break: break-word; }
.shots { display: flex; flex-wrap: wrap; gap: 20px; margin-top: 12px; }
figure { margin: 0; padding: 8px; background: #f6f8fa; border: 1px solid #d8dee4; border-radius: 8px; break-inside: avoid; }
figure img { display: block; max-width: 100%; width: 360px; border: 1px solid #d1d9e0; border-radius: 4px; background: #fff; }
figcaption { margin-top: 6px; font-size: 12px; color: #59636e; word-break: break-all; }
.overall { background: #f6f8fa; border: 1px solid #d8dee4; border-radius: 6px; padding: 12px 16px; white-space: pre-wrap; }
footer { margin-top: 32px; padding-top: 10px; border-top: 1px solid #d8dee4; font-size: 12px; color: #6e7781; text-align: right; }
@media print { body { background: #fff; } .page { box-shadow: none; margin: 0; padding: 0; max-width: none; border-radius: 0; } a { color: inherit; } }
@media (max-width: 600px) { .page { padding: 20px 16px; } .stats { grid-template-columns: 1fr; } }
</style></head><body><main class="page">
<header>
<div class="kicker">${esc(l.title)}</div>
<h1>${esc(meta.project)}</h1>
<dl class="meta">
<div><dt>${esc(l.project)}</dt><dd>${esc(meta.project)}</dd></div>
<div><dt>${esc(l.date)}</dt><dd>${esc(meta.date)}</dd></div>${report?.project_type ? `\n<div><dt>${esc(l.projectType)}</dt><dd>${esc(report.project_type)}</dd></div>` : ""}
</dl>
</header>
${report ? `<div class="verdict ${verdictPass ? "pass" : "fail"}"><span>${verdictPass ? "✓" : "✗"} ${esc(verdictPass ? l.verdictPass : l.verdictFail)}</span><small>${esc(l.conclusion)} · ${esc(l.summary(count("pass"), count("fail"), count("manual")))}</small></div>` : ""}
<div class="stats">${stat("pass")}${stat("fail")}${stat("manual")}</div>
${bar}
${total ? `<h2>${esc(l.overview)}</h2>
<table><thead><tr><th class="no">#</th><th>${esc(l.criterion)}</th><th class="res">${esc(l.result)}</th></tr></thead><tbody>
${overview}
</tbody></table>
<h2>${esc(l.detail)}</h2>
${body}` : ""}
${report?.notes ? `<h2>${esc(l.overall)}</h2><div class="overall">${esc(report.notes)}</div>` : ""}
<footer>${esc(l.generatedBy)} · ${esc(meta.date)}</footer>
</main></body></html>
`;
}
