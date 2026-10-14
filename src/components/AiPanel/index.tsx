import {
  useEffect, useRef, useState, useCallback,
} from "react";
import { describeExit } from "../../lib/agentStepReport";
import { remoteSessionPromptNote } from "../../lib/remoteSession";
import { readFileAsAttachment } from "../../types/attachment";
import type { Attachment } from "../../types/attachment";
import { useMcpChat, TERMINAL_TOOL_NAME } from "../../hooks/useMcpChat";
import type { PanelMode } from "./ModeHint";
import type { ToolScope } from "../../ipc/ai";
import { repairUnterminatedHeredocs } from "../../lib/heredocGuard";
import { getPtyRecentOutput, writePty } from "../../ipc/pty";
import { getConfig, type SubmitShortcut } from "../../ipc/config";
import { getMcpTools } from "../../ipc/mcp";
import { languageDirective } from "../../lib/i18n";
import { useLocale } from "../../contexts/LocaleContext";
import { QuotaBadge } from "../QuotaBadge";
import { useProviderQuota } from "../../hooks/useProviderQuota";
import type { TerminalBlock } from "../../hooks/useTerminalBlocks";
import { WrenchIcon } from "../Icons";
import { ChatPanelShell } from "../ChatPanel/ChatPanelShell";

const IS_WINDOWS = navigator.platform.toLowerCase().startsWith("win");

const STORAGE_AGENT_MODE_KEY = "aiterm-agent-mode";
const STORAGE_USE_MCP_KEY = "aiterm-use-mcp";

/**
 * 指令跑著卻完全沒有輸出多久，就當它可能卡住了。
 *
 * 用「安靜多久」而不是「跑多久」當判準：使用者會跑弱掃、建置這類長工作，
 * 那些會持續吐輸出；真正卡住（heredoc 等收尾、互動程式等輸入）則是全然安靜。
 * 這個值寧可寬鬆——誤判的代價是打擾使用者，而漏判只是回到原本要按停止鈕的狀態。
 */
const STUCK_IDLE_MS = 120_000;
/** 多久檢查一次 getIdleMs()——不用太密，卡住偵測本來就是寬鬆判準。 */
const STUCK_CHECK_INTERVAL_MS = 5_000;

function loadSavedAgentMode(): boolean {
  try {
    const v = localStorage.getItem(STORAGE_AGENT_MODE_KEY);
    if (v !== null) return v === "true";
  } catch { /* ignore */ }
  return false;
}

function loadSavedUseMcp(): boolean {
  try {
    const v = localStorage.getItem(STORAGE_USE_MCP_KEY);
    if (v !== null) return v === "true";
  } catch { /* ignore */ }
  return true;
}

export interface AiPanelProps {
  sessionId: string;
  isOpen: boolean;
  providerName: string;
  /** 用來查配額。顯示名稱查不了——後端是用 id 找設定的。 */
  providerId?: string;
  onClose: () => void;
  onExecuteCommand: (cmd: string, onComplete?: (block: TerminalBlock) => void) => void;
  onOpenProviderPalette: () => void;
  sendRemoteResponse?: (text: string) => void;
  /** 距離 PTY 最後一次有輸出過了多久（毫秒）。用來偵測指令是不是卡住了。 */
  getIdleMs?: () => number;
  /** 這個分頁目前若在 ssh／telnet 進去的遠端 shell 裡，回傳目標主機。每一步都重讀，
   *  因為 agent 可能就是在這個任務中途才連進去的。 */
  getRemoteSession?: () => string | null;
  /** 中斷目前這個卡住的指令：送 Ctrl+C 並強制結案。 */
  onInterruptCommand?: () => void;
  /**
   * 使用者按了停止鍵。外層若代 agent 扣著指令的完成 callback（例如等使用者
   * 回應提權橫幅，見 TerminalView 的 heldCompletionRef），要在這裡立刻放行
   * ——光送 Ctrl+C 不夠，那時 shell 早就回到提示字元，沒有指令會因此結束。
   */
  onAgentAborted?: () => void;
}

/**
 * The panel stays mounted across open/close so `useMcpChat`'s event listener
 * keeps receiving streaming chunks while the user toggles Ctrl+I. We hide
 * the panel with a CSS class when `isOpen=false` rather than returning null.
 */
export function AiPanel({
  sessionId,
  isOpen,
  providerName,
  providerId,
  onClose,
  onExecuteCommand,
  onOpenProviderPalette,
  sendRemoteResponse,
  getIdleMs,
  getRemoteSession,
  onInterruptCommand,
  onAgentAborted,
}: AiPanelProps) {
  /** 常駐配額徽章的代表窗；null 就不顯示。 */
  const quotaWindow = useProviderQuota(providerId);
  const { t, locale } = useLocale();
  // 系統設定的「最大步數」（0 = 無限制，載入時已換成大數）。同時管 Agent 迴圈與 MCP 工具呼叫。
  const [maxAgentSteps, setMaxAgentSteps] = useState<number>(5);

  // ── 卡住偵測 ───────────────────────────────────────────────────────────────
  const [stuckPromptVisible, setStuckPromptVisible] = useState(false);
  const stuckIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  // 使用者按過「繼續等待」之前不再提示——否則下一次檢查會立刻又跳出來。
  const snoozeUntilRef = useRef(0);

  useEffect(() => {
    return () => {
      if (stuckIntervalRef.current !== null) {
        clearInterval(stuckIntervalRef.current);
        stuckIntervalRef.current = null;
      }
    };
  }, []);

  const clearStuckWatch = useCallback(() => {
    if (stuckIntervalRef.current !== null) {
      clearInterval(stuckIntervalRef.current);
      stuckIntervalRef.current = null;
    }
    setStuckPromptVisible(false);
  }, []);

  const handleStuckWait = useCallback(() => {
    snoozeUntilRef.current = Date.now() + STUCK_IDLE_MS;
    setStuckPromptVisible(false);
  }, []);

  const handleStuckInterrupt = useCallback(() => {
    // 只送中斷，不自己 resolve 等待中的 Promise——強制結案會觸發既有的完成
    // callback，Promise 自然 resolve，避免雙重 resolve 造成狀態不一致。
    onInterruptCommand?.();
  }, [onInterruptCommand]);

  // ⚡ 模式的「執行終端機指令」工具（run_command）：onExecuteCommand → 終端機 → 完成回呼。
  // 風險閘門在 useMcpChat 呼叫它之前已經過了（見 terminalCommandGate）。
  const terminalRunningRef = useRef(false);
  // 每次執行指令的編號：停止後舊指令的完成回呼可能很晚才到（例如外層扣著它等提權橫幅），
  // 那時若已有新指令在跑，舊回呼不可以清掉新指令的執行旗標與卡住偵測。
  const terminalRunIdRef = useRef(0);
  const runTerminalCommand = useCallback(
    (rawCommand: string) =>
      new Promise<string>((resolve) => {
        // 見 heredocGuard.ts：缺結束標記的 heredoc 會讓 shell 停在 heredoc>，指令永遠不結束。
        const cmd = repairUnterminatedHeredocs(rawCommand.trim());
        const runId = ++terminalRunIdRef.current;
        terminalRunningRef.current = true;
        clearStuckWatch();
        snoozeUntilRef.current = 0;
        // 指令跑著卻完全沒有輸出多久，就提示使用者可能卡住了（見 STUCK_IDLE_MS）。
        // 這個 interval 只在等這個指令跑完的期間存在：完成、停止或卸載時都會清掉。
        if (getIdleMs) {
          stuckIntervalRef.current = setInterval(() => {
            if (Date.now() < snoozeUntilRef.current) return;
            if (getIdleMs() >= STUCK_IDLE_MS) setStuckPromptVisible(true);
          }, STUCK_CHECK_INTERVAL_MS);
        }
        onExecuteCommand(cmd, async (block) => {
          if (terminalRunIdRef.current === runId) {
            terminalRunningRef.current = false;
            clearStuckWatch();
          }
          const rawOutput = (await getPtyRecentOutput(sessionId).catch(() => null)) ?? block.rawOutput ?? "";
          resolve(
            `Command \`${cmd}\` finished (${describeExit(block)}).\nOutput:\n\`\`\`\n${rawOutput.slice(-2000)}\n\`\`\``,
          );
        });
      }),
    [onExecuteCommand, sessionId, getIdleMs, clearStuckWatch],
  );
  const chat = useMcpChat(sessionId, maxAgentSteps, runTerminalCommand);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const MAX_IMAGE_BYTES = 5 * 1024 * 1024; // 5 MB
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [mcpEnabled, setMcpEnabled] = useState(true);
  const [mcpToolCount, setMcpToolCount] = useState(0);
  const [useMcp, setUseMcp] = useState(loadSavedUseMcp);
  const [submitShortcut, setSubmitShortcut] = useState<SubmitShortcut>("enter");

  const processFiles = useCallback(async (files: FileList | File[]) => {
    const arr = Array.from(files);
    const results = await Promise.allSettled(arr.map(async (file) => {
      if (file.type.startsWith("image/") && file.size > MAX_IMAGE_BYTES) {
        throw new Error(`${file.name} 超過 5MB 限制`);
      }
      return readFileAsAttachment(file);
    }));
    const valid = results
      .filter((r): r is PromiseFulfilledResult<Attachment> => r.status === "fulfilled")
      .map((r) => r.value);
    setAttachments((prev) => [...prev, ...valid]);
  }, [MAX_IMAGE_BYTES]);

  const handlePaste = useCallback(async (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const files = e.clipboardData?.files;
    if (files && files.length > 0) {
      e.preventDefault();
      await processFiles(files);
    }
    // No files → let default text paste proceed
  }, [processFiles]);

  const handleDragOver = useCallback((e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
  }, []);

  const handleDrop = useCallback(async (e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) {
      await processFiles(files);
    }
  }, [processFiles]);

  // ── Agent mode ────────────────────────────────────────────────────────────
  // ⚡ 決定 AI 能不能自己動手（run_command）；MCP 按鈕再決定動手時能不能用 MCP／電腦操作工具。
  const [agentMode, setAgentMode] = useState(loadSavedAgentMode);

  // 每次開啟都重讀：面板是常駐不卸載的，只在掛載時讀一次的話，使用者在設定
  // 裡改了 max_agent_steps（或裝了新的 MCP server），要重開 app 才會反映。
  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    const load = async () => {
      const [cfg, tools] = await Promise.all([getConfig(), getMcpTools()]);
      if (cancelled) return;
      // 0 = unlimited; use a large number internally
      setMaxAgentSteps(cfg.max_agent_steps === 0 ? 9999 : (cfg.max_agent_steps ?? 5));
      const globalEnabled = cfg.mcp_enabled ?? true;
      setMcpEnabled(globalEnabled);
      // 內建的終端機工具就是 ⚡ 本身，不算「MCP 工具」。
      setMcpToolCount(tools.filter((tool) => tool.name !== TERMINAL_TOOL_NAME).length);
      setSubmitShortcut(cfg.submit_shortcut ?? "enter");
    };
    load().catch(() => {});
    return () => { cancelled = true; };
  }, [isOpen]);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_AGENT_MODE_KEY, String(agentMode));
    } catch { /* ignore */ }
  }, [agentMode]);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_USE_MCP_KEY, String(useMcp));
    } catch { /* ignore */ }
  }, [useMcp]);

  /**
   * ⚡ 模式額外送給模型的規則：Windows 提權（exit 740）與遠端連線說明。只送給模型，不顯示。
   * cwd 與目錄內容由後端的 context::snapshot 放進系統提示詞，這裡不重複。
   */
  const buildAgentExtraPrompt = useCallback((): string => {
    // Windows 專屬：沒有這條規則時，AI 遇到 exit 740 會自己發明 `Start-Process -Verb RunAs`，
    // 把指令送進一個 AITerm 看不到輸出的獨立提權視窗。AITerm 自己會偵測 740 並顯示提權橫幅，
    // 使用者確認後在提權 session 重跑並把結果交回（見 TerminalView 的 heldCompletionRef）。
    const elevationRule = IS_WINDOWS
      ? `Elevation: if a command fails with exit code 740, or reports that it needs elevated/administrator privileges, do NOT try to work around it — never use \`Start-Process -Verb RunAs\`, \`runas\`, or redirect an elevated command's output to a temp file and read it back. Those run in a separate window whose output this terminal cannot see. AITerm handles elevation itself: when a command fails for lack of privileges it asks the user to elevate this session, and if they approve, it re-runs the command elevated and gives you that result instead. So if you still receive a privilege failure, the user declined elevation — stop calling aiterm_terminal__run_command, and tell the user in ${languageDirective(locale)} that the command needs administrator rights and that they can ask you again and approve AITerm's elevation prompt.`
      : "";
    return (elevationRule + remoteSessionPromptNote(getRemoteSession?.())).trim();
  }, [locale, getRemoteSession]);

  // Forward new assistant messages to Telegram
  const prevMessagesLength = useRef(chat.messages.length);
  useEffect(() => {
    if (chat.messages.length > prevMessagesLength.current) {
      const lastMsg = chat.messages[chat.messages.length - 1];
      if (lastMsg?.role === "assistant" && sendRemoteResponse && !chat.isStreaming) {
        const text = typeof lastMsg.content === "string" ? lastMsg.content : Array.isArray(lastMsg.content) ? lastMsg.content.filter((p) => p.type === "text").map((p) => (p as { type: "text"; text: string }).text).join(" ") : "";
        sendRemoteResponse(text);
      }
    }
    prevMessagesLength.current = chat.messages.length;
  }, [chat.messages, chat.isStreaming, sendRemoteResponse]);

  // MCP 只在 ⚡ 開時有作用。送出時與模式說明列共用同一個判斷——拆成兩份寫的話
  // 遲早會有一邊漏改，畫面就會說謊。
  const mcpActive = agentMode && useMcp && mcpEnabled && mcpToolCount > 0;
  const mode: PanelMode = agentMode ? (mcpActive ? "agent-mcp" : "agent-terminal") : "suggest";
  // ⚡ 這一輪正在跑（有工具可用）：由狀態列接手（步數、階段、停止鈕）。
  const agentRunning = chat.activeScope === "terminal" || chat.activeScope === "all";
  const isDisabled = chat.isStreaming;

  const sendTurn = (text: string) => {
    const currentAttachments = attachments;
    setAttachments([]);
    const scope: ToolScope = agentMode ? (mcpActive ? "all" : "terminal") : "none";
    void chat.send(
      text,
      scope,
      undefined,
      currentAttachments.length > 0 ? currentAttachments : undefined,
      // 傳函式而不是字串：useMcpChat 每一步重算，任務中途 ssh 進遠端後下一步就會帶上說明。
      agentMode ? () => buildAgentExtraPrompt() || undefined : undefined,
    );
  };

  // 停止：中斷 AI 呼叫；有指令在跑就 Ctrl+C 並強制結案；並通知外層放行被扣住的完成回呼（提權橫幅）。
  const stopTurn = () => {
    chat.stop();
    if (terminalRunningRef.current) {
      terminalRunningRef.current = false;
      clearStuckWatch();
      if (onInterruptCommand) onInterruptCommand();
      else writePty(sessionId, "\x03").catch(() => {});
    }
    onAgentAborted?.();
  };

  return (
    <ChatPanelShell
      isOpen={isOpen}
      onClose={onClose}
      messages={chat.messages}
      streamBuf={chat.streamBuf}
      isStreaming={chat.isStreaming}
      // **兩個階段都要有指示**：等 AI 想、以及等指令跑完——後者原本對話框
      // 是全靜的，使用者只看到氣泡消失然後乾等，回報成「空檔很長」。
      thinkingLabel={
        agentRunning
          ? chat.phase === "thinking"
            ? t.ai_agent_thinking
            : t.ai_agent_executing
          : chat.isStreaming
            ? t.ai_thinking
            : null
      }
      error={chat.error}
      onRetry={chat.resend}
      onExecuteCommand={onExecuteCommand}
      agentMode={agentMode}
      onToggleAgentMode={() => setAgentMode((m) => !m)}
      onSend={sendTurn}
      onSubmitAgent={sendTurn}
      mode={mode}
      maxAgentSteps={maxAgentSteps}
      mcpToolCount={mcpToolCount}
      agentRunning={agentRunning}
      agentPhase={chat.phase}
      agentStep={chat.step}
      onStop={stopTurn}
      onAbortAgent={stopTurn}
      providerName={providerName}
      onOpenProviderPalette={onOpenProviderPalette}
      headerBadge={quotaWindow ? <QuotaBadge window={quotaWindow} /> : undefined}
      sessions={chat.sessions}
      onLoadSession={(s) => chat.loadMessages(s.messages, s.id)}
      onNewChat={() => chat.clear()}
      onDeleteSession={(id) => chat.deleteSession(id)}
      toolFallbackReason={chat.toolFallbackReason}
      submitShortcut={submitShortcut}
      allowEmptySubmit={attachments.length > 0}
      onPaste={handlePaste}
      isWindows={IS_WINDOWS}
      onDragOver={handleDragOver}
      onDrop={handleDrop}
      inputPrefixControls={
        <button
          type="button"
          className="aiterm-pill-paperclip-btn"
          onClick={() => fileInputRef.current?.click()}
          title={t.ai_attach_file}
          disabled={isDisabled}
        >
          📎
        </button>
      }
      extraInputControls={mcpEnabled ? (
        <button
          type="button"
          // MCP 只在 ⚡ 開時生效：⚡ 關＝純聊天，按鈕停用且不亮，但記住原本的開關（localStorage）。
          className={`aiterm-mcp-toggle${mcpActive ? " aiterm-mcp-toggle--on" : ""}`}
          title={
            !agentMode
              ? t.ai_mcp_needs_agent
              : mcpToolCount === 0
                ? t.mcp_toggle_no_servers
                : (useMcp ? t.ai_mcp_on : t.ai_mcp_off)
          }
          disabled={!agentMode || mcpToolCount === 0 || isDisabled}
          onClick={() => setUseMcp((v) => !v)}
        >
          <WrenchIcon size={12} />
          <span>{mcpToolCount > 0 ? `MCP (${mcpToolCount})` : "MCP OFF"}</span>
        </button>
      ) : null}
      extraAboveInput={
        <>
          {agentRunning && stuckPromptVisible && (
            <div className="aiterm-stuck-prompt" role="alert">
              <div className="aiterm-stuck-prompt__title">{t.agent_stuck_title}</div>
              <div className="aiterm-stuck-prompt__body">{t.agent_stuck_body}</div>
              <div className="aiterm-stuck-prompt__actions">
                <button
                  type="button"
                  className="aiterm-stuck-prompt__wait"
                  onClick={handleStuckWait}
                >
                  {t.agent_stuck_wait}
                </button>
                <button
                  type="button"
                  className="aiterm-stuck-prompt__interrupt"
                  onClick={handleStuckInterrupt}
                >
                  {t.agent_stuck_interrupt}
                </button>
              </div>
            </div>
          )}
          {attachments.length > 0 && (
            <div className="aiterm-attachment-pills">
              {attachments.map((att) => (
                <div key={att.id} className="aiterm-attachment-pill">
                  {att.previewUrl && (
                    <img src={att.previewUrl} alt={att.name} className="aiterm-pill-thumb" />
                  )}
                  <span className="aiterm-pill-name" title={att.name}>{att.name}</span>
                  <button
                    type="button"
                    className="aiterm-pill-remove"
                    onClick={() => setAttachments((prev) => prev.filter((a) => a.id !== att.id))}
                  >×</button>
                </div>
              ))}
            </div>
          )}
          <input
            ref={fileInputRef}
            type="file"
            multiple
            style={{ display: "none" }}
            onChange={(e) => { if (e.target.files) void processFiles(e.target.files); }}
          />
        </>
      }
    />
  );
}
