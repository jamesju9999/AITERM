import { useCallback, useEffect, useRef, useState } from "react";
import { abortAi, formatAiError, invokeAiChatCtx, type AiError } from "../../ipc/ai";
import { useLocale } from "../../contexts/LocaleContext";
import { languageDirective } from "../../lib/i18n";
import { fillTerminalInput, serializeTerminal, submitTerminalInput } from "../../lib/terminalInstanceRegistry";
import { buildSuggestionRequest, parseSuggestions, type PromptSuggestion } from "../../lib/promptSuggestions";
import { stripAnsiCodes } from "../TaskBoard/transcriptUtils";
import "./PromptSuggestions.css";

const STORAGE_AUTO_KEY = "aiterm-suggest-auto";
/** 終端機超過這麼久沒有輸出，才算閒置（Claude 回完了）。 */
const IDLE_MS = 2_000;
const POLL_MS = 1_000;

type Status = "idle" | "loading" | "ok" | "none" | "empty" | "error";

function loadAuto(): boolean {
  try { return localStorage.getItem(STORAGE_AUTO_KEY) === "true"; } catch { return false; }
}

/** Tauri 的錯誤是物件而不是 Error——不能 String(e)，會變成 [object Object]。 */
function describeError(e: unknown): string {
  if (e && typeof e === "object") {
    if ("kind" in e) return formatAiError(e as AiError);
    if ("message" in e) return String((e as { message: unknown }).message);
  }
  return typeof e === "string" ? e : "unknown";
}

export interface PromptSuggestionsProps {
  sessionId: string;
  providerId?: string;
  /** Ask AI 串流中或 Agent 執行中——這時不產生也不可按。 */
  disabled: boolean;
  /** 距離 PTY 最後一次輸出多久（毫秒）。沒給就當作永遠閒置。 */
  getIdleMs?: () => number;
}

export function PromptSuggestions({ sessionId, providerId, disabled, getIdleMs }: PromptSuggestionsProps) {
  const { t, locale } = useLocale();
  const [items, setItems] = useState<PromptSuggestion[]>([]);
  const [status, setStatus] = useState<Status>("idle");
  const [errorMsg, setErrorMsg] = useState("");
  const [auto, setAuto] = useState(loadAuto);
  const [terminalBusy, setTerminalBusy] = useState(() => (getIdleMs ? getIdleMs() < IDLE_MS : false));

  const connId = `suggest-${sessionId}`;
  const mountedRef = useRef(true);
  const reqRef = useRef(0);
  const loadingRef = useRef(false);
  const lastScreenRef = useRef<string | null>(null);
  const wasBusyRef = useRef(false);
  const getIdleMsRef = useRef(getIdleMs);
  useEffect(() => { getIdleMsRef.current = getIdleMs; }, [getIdleMs]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      // 卸載時還在等回應就取消，免得白燒額度。
      if (loadingRef.current) abortAi(connId).catch(() => {});
    };
  }, [connId]);

  // 用輪詢而不是事件：getIdleMs 只是個讀 ref 的函式，沒有可訂閱的來源。
  useEffect(() => {
    const id = setInterval(() => {
      const fn = getIdleMsRef.current;
      setTerminalBusy(fn ? fn() < IDLE_MS : false);
    }, POLL_MS);
    return () => clearInterval(id);
  }, []);

  const generate = useCallback(async (isAuto: boolean) => {
    const screen = stripAnsiCodes(serializeTerminal(sessionId) ?? "");
    if (!screen.trim()) {
      if (!isAuto) { setItems([]); setStatus("empty"); }
      return;
    }
    // 自動模式不對同一份畫面重複花額度；手動按就一定要產生。
    if (isAuto && screen === lastScreenRef.current) return;
    lastScreenRef.current = screen;

    if (loadingRef.current) abortAi(connId).catch(() => {});
    const myReq = ++reqRef.current;
    loadingRef.current = true;
    setStatus("loading");
    try {
      const reply = await invokeAiChatCtx(
        [{ role: "user", content: buildSuggestionRequest(screen, languageDirective(locale)) }],
        { os: navigator.platform.toLowerCase(), shell: null, cwd: null, recentOutput: null },
        connId,
        providerId,
        locale,
      );
      if (!mountedRef.current || myReq !== reqRef.current) return;
      const parsed = parseSuggestions(reply.content);
      setItems(parsed);
      setStatus(parsed.length > 0 ? "ok" : "none");
    } catch (e) {
      if (!mountedRef.current || myReq !== reqRef.current) return;
      setItems([]);
      setErrorMsg(describeError(e));
      setStatus("error");
    } finally {
      if (myReq === reqRef.current) loadingRef.current = false;
    }
  }, [sessionId, connId, providerId, locale]);

  // 忙→閒的那一下才自動產生；wasBusyRef 確保一次忙碌只觸發一次。
  useEffect(() => {
    if (terminalBusy) { wasBusyRef.current = true; return; }
    if (wasBusyRef.current && auto && !disabled) {
      wasBusyRef.current = false;
      void generate(true);
    }
  }, [terminalBusy, auto, disabled, generate]);

  const toggleAuto = () => {
    const next = !auto;
    setAuto(next);
    try { localStorage.setItem(STORAGE_AUTO_KEY, String(next)); } catch { /* ignore */ }
  };

  const blocked = disabled || terminalBusy;
  const hasRun = status !== "idle";
  const hint = status === "loading" ? t.suggest_loading
    : terminalBusy ? t.suggest_busy
    : status === "empty" ? t.suggest_empty_screen
    : status === "none" ? t.suggest_none
    : null;

  return (
    <div className="aiterm-suggest">
      <div className="aiterm-suggest__bar">
        <button
          type="button"
          className="aiterm-suggest__gen"
          disabled={blocked}
          onClick={() => void generate(false)}
        >
          {hasRun ? t.suggest_regenerate : t.suggest_generate}
        </button>
        <button
          type="button"
          className={`aiterm-suggest__auto${auto ? " aiterm-suggest__auto--on" : ""}`}
          aria-pressed={auto}
          title={t.suggest_auto_title}
          onClick={toggleAuto}
        >
          {t.suggest_auto}
        </button>
        {hint && <span className="aiterm-suggest__hint">{hint}</span>}
      </div>
      {status === "error" && (
        <div className="aiterm-suggest__error" role="alert">
          <span>{t.suggest_error(errorMsg)}</span>
          <button type="button" disabled={blocked} onClick={() => void generate(false)}>
            {t.suggest_retry}
          </button>
        </div>
      )}
      {items.length > 0 && (
        <div className="aiterm-suggest__cards">
          {items.map((s) => (
            <button
              key={s.prompt}
              type="button"
              className="aiterm-suggest__card"
              title={`${t.suggest_fill_title}\n\n${s.prompt}`}
              // 單擊＝填入供編輯；雙擊＝送出。雙擊會先觸發一次單擊（detail=1），
              // 文字那時已經填好了，所以第二下（detail>=2）只補一個 Enter——
              // 再填一次就會貼兩遍。
              onClick={(e) => {
                if (e.detail >= 2) void submitTerminalInput(sessionId);
                else void fillTerminalInput(sessionId, s.prompt);
              }}
            >
              {s.title}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
