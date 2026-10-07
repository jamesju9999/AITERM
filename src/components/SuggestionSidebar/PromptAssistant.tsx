import { useEffect, useRef, useState } from "react";
import { abortAi, invokeAiChatCtx } from "../../ipc/ai";
import { useLocale } from "../../contexts/LocaleContext";
import { languageDirective } from "../../lib/i18n";
import { redactSecrets } from "../../lib/redact";
import {
  MAX_ASSIST_PROMPT_CHARS,
  MAX_ASSIST_REQUEST_CHARS,
  buildPromptAssistRequest,
  cleanGeneratedText,
} from "../../lib/promptSuggestions";
import { normalizeScreen } from "../../lib/screenHistory";
import { fillTerminalInput, serializeTerminal, submitTerminalInput } from "../../lib/terminalInstanceRegistry";
import { describeError } from "./describeError";
import "./PromptAssistant.css";

export interface PromptAssistantProps {
  sessionId: string;
  providerId?: string;
  /** 大目標；沒有就是空字串。 */
  goal: string;
  /** 已整理好的里程碑一節（formatMilestonesForPrompt）；沒有就是空字串。 */
  milestoneContext: string;
  /** 送出畫面給 AI 前先遮罩敏感資訊。使用者自己輸入的需求不遮罩。 */
  redact: boolean;
}

/**
 * 提示詞助手：使用者用一句話寫粗略需求，AI 改寫成清楚具體、可直接貼給 AI 命令列工具的提示詞。
 *
 * 結果只放進可編輯的框裡，使用者按「填入」或「填入並送出」才會動到終端機——AI 不會自己送東西進終端機。
 * 草稿與結果只在記憶體，不持久化。
 */
export function PromptAssistant({ sessionId, providerId, goal, milestoneContext, redact }: PromptAssistantProps) {
  const { t, locale } = useLocale();
  const [request, setRequest] = useState("");
  const [result, setResult] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [redactedCount, setRedactedCount] = useState(0);

  const reqRef = useRef(0);
  const busyRef = useRef(false);
  const connId = `prompt-assist-${sessionId}`;

  /** 讓還在路上的回應過期並取消請求。 */
  const cancelInFlight = () => {
    reqRef.current += 1;
    if (busyRef.current) { busyRef.current = false; abortAi(connId).catch(() => {}); }
    setBusy(false);
  };
  useEffect(() => () => {
    if (busyRef.current) abortAi(connId).catch(() => {});
  }, [connId]);

  const run = async () => {
    const need = request.trim();
    if (!need || busyRef.current) return;
    const myReq = ++reqRef.current;
    busyRef.current = true;
    setBusy(true);
    setError(null);

    // 先遮罩、再截尾（buildPromptAssistRequest 會取畫面尾端）：截斷若切在 token 中間會留下殘片。
    const raw = normalizeScreen(serializeTerminal(sessionId) ?? "");
    let screen = raw;
    let masked = 0;
    if (raw && redact) {
      const r = redactSecrets(raw);
      screen = r.text;
      masked = r.count;
    }
    setRedactedCount(masked);

    try {
      const reply = await invokeAiChatCtx(
        [{ role: "user", content: buildPromptAssistRequest({ request: need, languageDirective: languageDirective(locale), goal, milestones: milestoneContext, screen }) }],
        { os: navigator.platform.toLowerCase(), shell: null, cwd: null, recentOutput: null },
        connId,
        providerId,
        locale,
      );
      if (myReq !== reqRef.current) return;
      const prompt = cleanGeneratedText(reply.content, MAX_ASSIST_PROMPT_CHARS);
      if (!prompt) setError(t.pa_empty);
      else setResult(prompt);
    } catch (e) {
      if (myReq !== reqRef.current) return;
      setError(t.pa_error(describeError(e)));
    } finally {
      if (myReq === reqRef.current) { busyRef.current = false; setBusy(false); }
    }
  };

  const clear = () => {
    cancelInFlight();
    setRequest("");
    setResult(null);
    setError(null);
    setRedactedCount(0);
  };

  const fill = async (alsoSend: boolean) => {
    // 空白時兩個按鈕都是 disabled，這裡不會被呼叫。
    const ok = await fillTerminalInput(sessionId, (result ?? "").trim());
    if (ok && alsoSend) await submitTerminalInput(sessionId);
  };

  const blank = !(result ?? "").trim();
  return (
    <section className="aiterm-pa" aria-label={t.pa_title}>
      {/* 結果在輸入列上方展開，像聊天軟體的輸入框：輸入列永遠在最下面、不用捲動就找得到。 */}
      {result !== null && (
        <div className="aiterm-pa__result">
          <textarea
            aria-label={t.pa_result_label}
            value={result}
            onChange={(e) => setResult(e.target.value)}
            maxLength={MAX_ASSIST_PROMPT_CHARS}
            rows={6}
          />
          <div className="aiterm-pa__actions">
            <button type="button" className="aiterm-pa__run" disabled={blank} onClick={() => void fill(false)}>{t.pa_fill}</button>
            <button type="button" className="aiterm-pa__plain" disabled={blank} onClick={() => void fill(true)}>{t.pa_fill_send}</button>
            <button type="button" className="aiterm-pa__plain" disabled={busy || !request.trim()} onClick={() => void run()}>{t.pa_again}</button>
          </div>
        </div>
      )}

      {error && <div className="aiterm-pa__error" role="alert">{error}</div>}
      {redactedCount > 0 && <div className="aiterm-pa__redacted">{t.suggest_redacted(redactedCount)}</div>}

      <div className="aiterm-pa__composer">
        <textarea
          aria-label={t.pa_request_label}
          value={request}
          onChange={(e) => setRequest(e.target.value)}
          maxLength={MAX_ASSIST_REQUEST_CHARS}
          rows={2}
          placeholder={t.pa_request_placeholder}
          title={t.pa_hint}
        />
        <div className="aiterm-pa__actions">
          <button type="button" className="aiterm-pa__run" disabled={!request.trim() || busy} onClick={() => void run()}>
            {busy ? t.pa_running : t.pa_run}
          </button>
          {(result !== null || request) && (
            <button type="button" className="aiterm-pa__plain" onClick={clear}>{t.pa_clear}</button>
          )}
        </div>
        {result === null && <p className="aiterm-pa__hint">{t.pa_hint}</p>}
      </div>
    </section>
  );
}
