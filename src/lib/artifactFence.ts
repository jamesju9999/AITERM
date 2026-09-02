import type { ArtifactKind } from "../contexts/ArtifactPanelContext";

export interface SplitArtifact {
  /** fence 之前的一般回覆文字，照常走 markdown 渲染。 */
  prose: string;
  /** 找不到 artifact fence 時為 null。 */
  artifact: { kind: ArtifactKind; content: string } | null;
}

const FENCE = /^```artifact-(html|chart)[ \t]*$/m;

/**
 * 把回覆文字拆成「fence 前的說明」與「artifact 內容」。
 *
 * 刻意不交給 markdown 的 fenced code block 規則去解析：那個規則遇到內容裡出現
 * ``` 就會提前收尾。實機上模型寫了一份含 ``` 的長 HTML 報告，結果面板只拿到
 * 前半段（文件被截斷），後半段溢出成聊天泡泡裡的一大坨原始 HTML。
 *
 * 規則因此定為「開頭之後全部都算文件內容，直到訊息結束」，只把結尾那一個
 * 收尾 fence 去掉。這跟系統提示給模型的契約一致：說明寫在前面，一則訊息最多
 * 一個 artifact。抽取發生在 markdown 解析之前，跟這個 repo 既有的
 * `parseCmdTags` 是同一個模式。
 */
export function splitArtifactFence(text: string): SplitArtifact {
  // 模型輸出在 Windows 上可能帶 CRLF。FENCE 以及底下的收尾正則都用 \n 當行界，
  // 殘留的 \r 會卡在 `[ \t]*$` 前面，讓 ```artifact-html 那一行比對不到——結果
  // 整個 artifact 沒被辨識出來，右側面板完全不會開。先把行尾正規化成 \n。
  text = text.replace(/\r\n/g, "\n");

  const m = FENCE.exec(text);
  if (!m || m.index === undefined) return splitBareHtmlDocument(text);

  const kind = m[1] === "chart" ? "chart" : "html";
  const prose = text.slice(0, m.index).trimEnd();

  const afterFence = text.slice(m.index + m[0].length);
  // 去掉緊接在 fence 語言標記後面的那個換行。
  let content = afterFence.startsWith("\n") ? afterFence.slice(1) : afterFence;

  if (kind === "chart") {
    // 圖表在第一個收尾 fence 就停。內容是 JSON，不會合法地含有「一整行 ```」，
    // 而模型很常在 fence 後面再補一句說明——沿用 html 那套吃到結尾的規則，
    // 那句話會被塞進 JSON 裡讓 parse 直接失敗，面板只會顯示「格式錯誤」。
    const close = content.search(/^```[ \t]*$/m);
    if (close >= 0) content = content.slice(0, close);
    content = content.replace(/\n+$/, "");
  } else {
    // HTML 文件相反：它合法地可能含有 ```（模型在報告裡放程式碼範例），所以
    // 只去掉「訊息最尾端」那個收尾 fence，中間出現的一律當文件內容。
    content = content.replace(/\n?```[ \t]*\n?$/, "");
  }

  return { prose, artifact: { kind, content } };
}

/** 文件開頭的形狀：`<!DOCTYPE html …>` 或 `<html …>`（`<htmlfoo` 不算）。 */
const BARE_HTML_START = /^\s*(?:<!doctype\s+html|<html[\s>])/i;

/**
 * 保底：模型沒照協定包 ```artifact-html，直接把一份完整 HTML 文件當回覆內文
 * 吐出來時，仍然把它抽成 html artifact。實測較弱的模型（KB 問答 + 本地模型，
 * 尤其剛安裝後第一次冷啟動）會這樣，前端否則只顯示一堆裸標籤、面板也不開。
 *
 * 邊界刻意收得很緊——只認「整則回覆一開頭就是文件」：
 *  - 「回答裡在教 HTML」一定是先講一句話、再放 ``` code fence，開頭不會是裸
 *    doctype，所以那種回覆不受影響（FENCE 之外，這裡的開頭比對也擋掉）。
 *  - 文件之後模型常補一句「文件已產生…」，那段留作 prose。
 *  - 串流中 </html> 還沒到時，比照 fence 的做法：doctype/<html> 一出現就先
 *    當 in-progress artifact（markdown.tsx 會顯示「產生中」卡片）。
 */
function splitBareHtmlDocument(text: string): SplitArtifact {
  if (!BARE_HTML_START.test(text)) return { prose: text, artifact: null };

  const startIdx = text.search(/<!doctype\s+html|<html[\s>]/i);
  const closeMatch = /<\/html\s*>/i.exec(text.slice(startIdx));
  const endIdx = closeMatch
    ? startIdx + closeMatch.index + closeMatch[0].length
    : text.length;

  const content = text.slice(startIdx, endIdx).trim();
  const prose = (text.slice(0, startIdx) + text.slice(endIdx)).trim();

  return { prose, artifact: { kind: "html", content } };
}
