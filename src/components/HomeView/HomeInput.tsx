import { useRef, useState } from "react";
import { useLocale } from "../../contexts/LocaleContext";
import { visibleTabCatalog } from "../NewTabPicker/tabCatalog";
import { invokeAiChat } from "../../ipc/ai";
import { parseRouteReply, fallbackRoute, type RouteResult } from "./routeIntent";

interface Props {
  onRoute: (result: RouteResult) => void;
}

/** 首頁最上面的自然語言輸入框：使用者打一句話，AI 判斷該開哪一種分頁。
 *  只在使用者按 Enter 時才呼叫 AI——首頁每次顯示都重新掛載，掛載時打 AI
 *  會變成一顆自動打點的按鈕。 */
export function HomeInput({ onRoute }: Props) {
  const { t } = useLocale();
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  // 穩定的 sessionId：整個輸入框的生命週期用同一個，不必每次送出都重新產生。
  const sessionIdRef = useRef(crypto.randomUUID());

  async function submit() {
    const userText = value.trim();
    if (!userText || busy) return;

    setBusy(true);
    const catalog = visibleTabCatalog(t)
      .map((e) => `${e.type}: ${e.label} — ${e.desc}`)
      .join("\n");
    const prompt =
      `使用者輸入了一句話，你要判斷該開啟哪一種分頁來處理。可選的分頁類型：\n${catalog}\n\n` +
      `使用者輸入：${userText}\n\n` +
      `只回一個 JSON 物件，格式為 {"type":"<上面清單中的一個分頁類型>"}，不要有其他文字。`;

    try {
      const reply = await invokeAiChat(
        [{ role: "user", content: prompt }],
        sessionIdRef.current,
      );
      onRoute(parseRouteReply(reply.content ?? "", userText));
    } catch {
      // AiError 每一種 kind 對使用者的意義都一樣：AI 這條路走不通，降級即可，
      // 不需要在這裡分流出不同訊息。
      onRoute(fallbackRoute(userText));
    } finally {
      setBusy(false);
      setValue("");
    }
  }

  return (
    <div className="home-input">
      <input
        type="text"
        className="home-input-box"
        value={value}
        disabled={busy}
        placeholder={t.home_input_placeholder}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") submit();
        }}
      />
    </div>
  );
}
