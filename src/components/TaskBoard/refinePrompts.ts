/**
 * 「AI 潤飾」的提示詞與回覆解析。
 *
 * 這一欄的內容最後會被原封不動送給 Claude Code 執行（見
 * `dispatch::build_prompt`），所以潤飾的目標不是把句子寫漂亮，而是把口語
 * 需求改寫成一份**可執行的任務指令**：目標、範圍、限制、驗收標準。
 */

export interface RefinedTask {
  /** 沒有要求標題、或模型沒給時是 null。 */
  title: string | null;
  body: string;
}

/** 模型用這一行把標題跟內容分開。 */
const SEPARATOR = "---";

export function buildRefinePrompt(body: string, projectDir: string, needTitle: boolean): string {
  const lines = [
    "你要把使用者草擬的一段工作需求，改寫成一份給 Claude Code 執行的任務指令。",
    "",
    "改寫原則：",
    "- 保留使用者的原意與所有具體資訊（檔名、路徑、數字、專有名詞），不要自己發明需求。",
    "- 補上結構：目標、範圍、限制條件、驗收標準。使用者沒講的部分不要編造，寧可省略。",
    "- 用祈使句直接寫指令，不要寫成對使用者說話的口吻。",
    "- 不要包成程式碼區塊，不要加開場白或結語。",
    "",
    `工作目錄：${projectDir || "（未指定）"}`,
    "",
  ];

  if (needTitle) {
    lines.push(
      "輸出格式（嚴格遵守）：",
      "第一行寫「標題：」加上一句不超過 20 字的簡短標題，",
      `第二行只寫 ${SEPARATOR}，`,
      "從第三行開始寫改寫後的工作內容。",
      "",
    );
  } else {
    lines.push("直接輸出改寫後的工作內容，不要有標題行。", "");
  }

  lines.push("使用者草擬的內容：", body);
  return lines.join("\n");
}

/**
 * 解析模型的回覆。
 *
 * 刻意不用 JSON：不是每個 provider 都支援 json mode（`supports_json_mode`
 * 是 false 的並不少），而 json_object 也只保證合法 JSON、不保證形狀。這裡
 * 的結構單純到用一行分隔符就夠，解析失敗的代價也只是少一個標題。
 *
 * 回傳 null 代表整份回覆是空的——那才是真的失敗。
 */
export function parseRefined(reply: string): RefinedTask | null {
  const text = stripFence(reply).trim();
  if (!text) return null;

  const lines = text.split("\n");
  const sepAt = lines.findIndex((l) => l.trim() === SEPARATOR);
  if (sepAt === -1) {
    // 沒有分隔線：模型沒照格式，或本來就沒要求標題。整份都當作內容。
    return { title: null, body: text };
  }

  const title = extractTitle(lines.slice(0, sepAt));
  const body = lines.slice(sepAt + 1).join("\n").trim();
  // 分隔線後面空的話，格式八成是壞的——保住內容比保住標題重要。
  if (!body) return { title: null, body: text };
  return { title, body };
}

/** 從分隔線前面的幾行裡找出標題。找不到就 null。 */
function extractTitle(head: string[]): string | null {
  for (const line of head) {
    const m = /^\s*(?:標題|title)\s*[:：]\s*(.+)$/i.exec(line);
    if (m) return m[1].trim() || null;
  }
  // 沒有「標題：」前綴，但分隔線前只有一行有字時，那一行就是標題。
  const nonEmpty = head.map((l) => l.trim()).filter(Boolean);
  return nonEmpty.length === 1 ? nonEmpty[0] : null;
}

/** 模型有時候還是會包 ```；剝掉最外層那一層。 */
function stripFence(text: string): string {
  const m = /^\s*```[^\n]*\n([\s\S]*?)\n?```\s*$/.exec(text);
  return m ? m[1] : text;
}
