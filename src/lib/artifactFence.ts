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
  const m = FENCE.exec(text);
  if (!m || m.index === undefined) return { prose: text, artifact: null };

  const kind = m[1] === "chart" ? "chart" : "html";
  const prose = text.slice(0, m.index).trimEnd();

  const afterFence = text.slice(m.index + m[0].length);
  // 去掉緊接在 fence 語言標記後面的那個換行。
  let content = afterFence.startsWith("\n") ? afterFence.slice(1) : afterFence;
  // 只去掉「訊息最尾端」的收尾 fence——中間出現的一律視為文件內容。
  content = content.replace(/\n?```[ \t]*\n?$/, "");

  return { prose, artifact: { kind, content } };
}
