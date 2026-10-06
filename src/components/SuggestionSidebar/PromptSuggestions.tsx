import { useCallback, useEffect, useRef, useState } from "react";
import { abortAi, invokeAiChatCtx } from "../../ipc/ai";
import { describeError } from "./describeError";
import { useLocale } from "../../contexts/LocaleContext";
import { languageDirective } from "../../lib/i18n";
import { fillTerminalInput, serializeTerminal, submitTerminalInput } from "../../lib/terminalInstanceRegistry";
import { buildSuggestionRequest, parseSuggestions, type PromptSuggestion } from "../../lib/promptSuggestions";
import { PROMPT_HISTORY_BUDGET, formatHistoryForPrompt } from "../../lib/screenHistory";
import { stripAnsiCodes } from "../TaskBoard/transcriptUtils";
import { RefreshIcon, SparklesIcon } from "../Icons";
import { IDLE_MS, POLL_MS } from "./terminalIdle";
import "./PromptSuggestions.css";

const STORAGE_AUTO_KEY = "aiterm-suggest-auto";
const STORAGE_COLLAPSED_KEY = "aiterm-suggest-collapsed";
/** 「已填入／已送出」回饋顯示多久。 */
const FLASH_MS = 1_500;

type Status = "idle" | "loading" | "ok" | "none" | "empty" | "error";

function loadAuto(): boolean {
  try { return localStorage.getItem(STORAGE_AUTO_KEY) === "true"; } catch { return false; }
}

function loadCollapsed(): boolean {
  try { return localStorage.getItem(STORAGE_COLLAPSED_KEY) === "true"; } catch { return false; }
}

export interface PromptSuggestionsProps {
  sessionId: string;
  providerId?: string;
  /** Ask AI 串流中或 Agent 執行中——這時不產生也不可按。 */
  disabled: boolean;
  /** 距離 PTY 最後一次輸出多久（毫秒）。沒給就當作永遠閒置。 */
  getIdleMs?: () => number;
  /** 側欄自己有標題與關閉鈕：不要再畫一個可收合的標題，內容永遠展開。 */
  hideTitle?: boolean;
  /** 使用者設定的大目標；有的話每個建議都會朝它推進。改了就丟掉舊目標產生的建議。 */
  goal?: string;
  /** 取得 AI 工具先前每一輪的「穩定畫面」（舊→新）。產生建議的當下才讀，所以傳函式不傳陣列。 */
  getHistory?: () => string[];
}

export function PromptSuggestions({ sessionId, providerId, disabled, getIdleMs, hideTitle = false, goal, getHistory }: PromptSuggestionsProps) {
  const { t, locale } = useLocale();
  const [items, setItems] = useState<PromptSuggestion[]>([]);
  const [status, setStatus] = useState<Status>("idle");
  const [errorMsg, setErrorMsg] = useState("");
  const [auto, setAuto] = useState(loadAuto);
  const [collapsed, setCollapsed] = useState(loadCollapsed);
  const [flash, setFlash] = useState<{ prompt: string; kind: "filled" | "sent" } | null>(null);
  const flashTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
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
      if (flashTimerRef.current) clearTimeout(flashTimerRef.current);
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

    const history = formatHistoryForPrompt(getHistory?.() ?? [], screen, PROMPT_HISTORY_BUDGET);

    if (loadingRef.current) abortAi(connId).catch(() => {});
    const myReq = ++reqRef.current;
    loadingRef.current = true;
    setStatus("loading");
    try {
      const reply = await invokeAiChatCtx(
        [{ role: "user", content: buildSuggestionRequest(screen, languageDirective(locale), goal, history) }],
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
  }, [sessionId, connId, providerId, locale, goal, getHistory]);

  // 目標一改，舊目標產生的建議就沒有意義了：丟掉卡片、取消還在跑的請求、
  // 回到起點，並讓自動模式對同一份畫面也能重新產生。第一次掛載不算「改」。
  const prevGoalRef = useRef(goal);
  useEffect(() => {
    if (prevGoalRef.current === goal) return;
    prevGoalRef.current = goal;
    reqRef.current += 1; // 讓還在路上的回應變成過期
    if (loadingRef.current) { loadingRef.current = false; abortAi(connId).catch(() => {}); }
    lastScreenRef.current = null;
    setItems([]);
    setStatus("idle");
    setErrorMsg("");
  }, [goal, connId]);

  // 忙→閒的那一下才自動產生；wasBusyRef 確保一次忙碌只觸發一次。
  useEffect(() => {
    if (terminalBusy) { wasBusyRef.current = true; return; }
    if (wasBusyRef.current && auto && !disabled) {
      wasBusyRef.current = false;
      void generate(true);
    }
  }, [terminalBusy, auto, disabled, generate]);

  const persist = (key: string, value: boolean) => {
    try { localStorage.setItem(key, String(value)); } catch { /* ignore */ }
  };
  const toggleAuto = () => { const next = !auto; setAuto(next); persist(STORAGE_AUTO_KEY, next); };
  const toggleCollapsed = () => { const next = !collapsed; setCollapsed(next); persist(STORAGE_COLLAPSED_KEY, next); };

  const showFlash = (prompt: string, kind: "filled" | "sent") => {
    if (flashTimerRef.current) clearTimeout(flashTimerRef.current);
    setFlash({ prompt, kind });
    flashTimerRef.current = setTimeout(() => setFlash(null), FLASH_MS);
  };

  // 單擊＝填入供編輯；雙擊＝送出。雙擊會先觸發一次單擊（detail=1），
  // 文字那時已經填好了，所以第二下（detail>=2）只補一個 Enter——
  // 再填一次就會貼兩遍。
  const onCardClick = (e: React.MouseEvent, prompt: string) => {
    if (e.detail >= 2) {
      void submitTerminalInput(sessionId);
      showFlash(prompt, "sent");
    } else {
      void fillTerminalInput(sessionId, prompt);
      showFlash(prompt, "filled");
    }
  };

  const isCollapsed = collapsed && !hideTitle;
  const blocked = disabled || terminalBusy;
  const hasRun = status !== "idle";
  const hint = status === "loading" ? null
    : terminalBusy ? t.suggest_busy
    : status === "empty" ? t.suggest_empty_screen
    : status === "none" ? t.suggest_none
    : null;

  return (
    <section className="aiterm-suggest" aria-label={t.suggest_title}>
      <header className="aiterm-suggest__head">
        {hideTitle ? <span /> : (
          <button
            type="button"
            className="aiterm-suggest__title"
            aria-expanded={!collapsed}
            onClick={toggleCollapsed}
          >
            <SparklesIcon size={13} />
            <span>{t.suggest_title}</span>
            {collapsed && items.length > 0 && <span className="aiterm-suggest__count">{items.length}</span>}
            <span className={`aiterm-suggest__chevron${collapsed ? "" : " aiterm-suggest__chevron--open"}`} aria-hidden="true">▸</span>
          </button>
        )}
        <div className="aiterm-suggest__tools">
          {hasRun && (
            <button
              type="button"
              className="aiterm-suggest__tool"
              disabled={blocked}
              onClick={() => void generate(false)}
            >
              <RefreshIcon size={11} />
              <span>{t.suggest_regenerate}</span>
            </button>
          )}
          <button
            type="button"
            role="switch"
            aria-checked={auto}
            className={`aiterm-suggest__switch${auto ? " aiterm-suggest__switch--on" : ""}`}
            title={t.suggest_auto_title}
            onClick={toggleAuto}
          >
            <span className="aiterm-suggest__track" aria-hidden="true"><span className="aiterm-suggest__thumb" /></span>
            <span>{t.suggest_auto}</span>
          </button>
        </div>
      </header>

      {!isCollapsed && (
        <div className="aiterm-suggest__body">
          {status === "idle" && (
            <div className="aiterm-suggest__intro">
              <p>
                {t.suggest_intro}
                <span className="aiterm-suggest__privacy">{t.suggest_privacy}</span>
              </p>
              <button
                type="button"
                className="aiterm-suggest__primary"
                disabled={blocked}
                onClick={() => void generate(false)}
              >
                <SparklesIcon size={13} />
                <span>{t.suggest_generate}</span>
              </button>
            </div>
          )}

          {status === "loading" && (
            <div className="aiterm-suggest__skeletons" role="status" aria-label={t.suggest_loading}>
              <span /><span /><span />
            </div>
          )}

          {hint && <div className="aiterm-suggest__hint">{hint}</div>}

          {status === "error" && (
            <div className="aiterm-suggest__error" role="alert">
              <span>{t.suggest_error(errorMsg)}</span>
              <button type="button" disabled={blocked} onClick={() => void generate(false)}>
                {t.suggest_retry}
              </button>
            </div>
          )}

          {items.length > 0 && status !== "loading" && (
            <>
              <ul className="aiterm-suggest__list">
                {items.map((s) => (
                  <li key={s.prompt}>
                    <button
                      type="button"
                      className="aiterm-suggest__card"
                      title={`${t.suggest_fill_title}\n\n${s.prompt}`}
                      onClick={(e) => onCardClick(e, s.prompt)}
                    >
                      <span className="aiterm-suggest__card-top">
                        <span className="aiterm-suggest__card-title">{s.title}</span>
                        {flash?.prompt === s.prompt && (
                          <span className="aiterm-suggest__badge">
                            <span aria-hidden="true">✓</span>
                            <span>{flash.kind === "sent" ? t.suggest_sent : t.suggest_filled}</span>
                          </span>
                        )}
                      </span>
                      <span className="aiterm-suggest__card-prompt">{s.prompt}</span>
                    </button>
                  </li>
                ))}
              </ul>
              <div className="aiterm-suggest__footer">{t.suggest_footer_hint}</div>
            </>
          )}
        </div>
      )}
    </section>
  );
}
