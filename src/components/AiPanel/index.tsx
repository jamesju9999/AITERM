import {
  useEffect, useRef, useState, useCallback,
} from "react";
import { readFileAsAttachment, contentToDisplayString } from "../../types/attachment";
import type { Attachment } from "../../types/attachment";
import { useMcpChat } from "../../hooks/useMcpChat";
import type { PanelMode } from "./ModeHint";
import { invokeAiChat, formatAiError, type AiError, type ChatMessage as AiChatMessage } from "../../ipc/ai";
import { getSessionCwd, listDirectory } from "../../ipc/fs";
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
  /** 中斷目前這個卡住的指令：送 Ctrl+C 並強制結案。 */
  onInterruptCommand?: () => void;
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
  onInterruptCommand,
}: AiPanelProps) {
  /** 常駐配額徽章的代表窗；null 就不顯示。 */
  const quotaWindow = useProviderQuota(providerId);
  const { t, locale } = useLocale();
  const chat = useMcpChat(sessionId);
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
  const [agentMode, setAgentMode] = useState(loadSavedAgentMode);
  const [agentRunning, setAgentRunning] = useState(false);
  const [agentStep, setAgentStep] = useState(0);
  // Agent 迴圈有兩個等待階段：等 AI 想下一步、等指令跑完。原本狀態列兩段
  // 顯示同一句話，看不出來卡在哪——尤其等 AI 那段完全沒有畫面變化。
  const [agentPhase, setAgentPhase] = useState<"thinking" | "running">("thinking");
  const agentAbortRef = useRef(false);
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

  const handleStuckWait = useCallback(() => {
    snoozeUntilRef.current = Date.now() + STUCK_IDLE_MS;
    setStuckPromptVisible(false);
  }, []);

  const handleStuckInterrupt = useCallback(() => {
    // 只送中斷，不自己 resolve 等待中的 Promise——強制結案會觸發既有的完成
    // callback，Promise 自然 resolve，避免雙重 resolve 造成狀態不一致。
    onInterruptCommand?.();
  }, [onInterruptCommand]);

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
      setMcpToolCount(tools.length);
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

  /** Build system prompt with live CWD + dir listing. */
  const buildAgentSystemPrompt = useCallback(async (): Promise<string> => {
    const cwd = await getSessionCwd(sessionId).catch(() => null) ?? "(unknown)";
    let dirList = "";
    try {
      const entries = await listDirectory(sessionId, "");
      dirList = entries
        .slice(0, 60)
        .map((e) => (e.is_dir ? `${e.name}/` : e.name))
        .join("\n");
    } catch { /* ignore */ }

    return `You are a terminal Agent. You can execute shell commands via <cmd>...</cmd> tags, and iterate based on the results to accomplish the user's goal.

Current working directory: ${cwd}
Directory contents (first 60 entries):
${dirList || "(unavailable)"}

Rules:
1. When you need to run a command, use <cmd>shell command</cmd> (only one at a time).
2. The system will execute it automatically and return the result — keep analyzing until the goal is achieved.
3. Once the goal is achieved, give your final explanation in ${languageDirective(locale)}, and do not include any more <cmd> tags.
4. Never perform destructive or irreversible operations (e.g. rm -rf /).
5. Write all explanations in ${languageDirective(locale)}.`;
  }, [sessionId, locale]);

  /**
   * Recursive agent loop. Each call:
   * 1. Asks AI with current history
   * 2. If AI returns a <cmd>, execute it and recurse with the output
   * 3. If no <cmd>, loop ends
   */
  // Holds the latest runAgentLoop so the recursive call below reads through
  // a ref instead of closing over the `const` being defined (which
  // react-hooks/immutability flags as accessed-before-declared).
  const runAgentLoopRef = useRef<
    (history: { role: "user" | "assistant"; content: string }[], systemPrompt: string, step: number) => Promise<void>
  >(async () => {});

  const runAgentLoop = useCallback(async (
    history: { role: "user" | "assistant"; content: string }[],
    systemPrompt: string,
    step: number,
  ): Promise<void> => {
    const maxSteps = maxAgentSteps;
    if (agentAbortRef.current || step >= maxSteps) {
      if (!agentAbortRef.current) {
        chat.addMessage({
          role: "assistant",
          content: locale === "zh-TW"
            ? `（Agent 已達最大步驟數 ${maxSteps}，停止迭代）`
            : `(Agent reached the max step count of ${maxSteps} and stopped)`,
        });
      }
      setAgentRunning(false);
      return;
    }

    setAgentStep(step + 1);
    setAgentPhase("thinking");

    // Ask AI
    let reply: string;
    try {
      const agentMessages: AiChatMessage[] = [
        { role: "system", content: systemPrompt },
        ...history,
      ];
      // supportsArtifacts=true：被要求「產生一份文件」時，模型若不知道有 artifact
      // 這條路，就會改用 shell 去寫檔案（cat > x.html <<'EOF' …），那個多行
      // heredoc 會讓終端機卡在 heredoc> 等不到結束標記。沒有 <cmd> 本來就代表
      // agent 完成，所以「吐出文件然後結束」正好是這個迴圈想要的收尾。
      const replyObj = await invokeAiChat(agentMessages, sessionId, undefined, false, locale, true);
      reply = replyObj.content ?? "";
    } catch (e) {
      // e may be an AiError object from Tauri IPC — use formatAiError to
      // produce a readable message instead of "[object Object]"
      const isAiError = e != null && typeof e === "object" && "kind" in (e as object);
      const errText = isAiError ? formatAiError(e as AiError) : String(e);
      chat.addMessage({
        role: "assistant",
        content: locale === "zh-TW"
          ? `（Agent 呼叫 AI 失敗，已停止：${errText}）`
          : `(Agent failed to call the AI and stopped: ${errText})`,
      });
      setAgentRunning(false);
      return;
    }

    if (agentAbortRef.current) { setAgentRunning(false); return; }

    // Show assistant reply in chat
    chat.addMessage({ role: "assistant", content: reply });
    if (sendRemoteResponse) sendRemoteResponse(reply);

    // Parse <cmd>
    const cmdMatch = reply.match(/<cmd>([\s\S]*?)<\/cmd>/i);
    if (!cmdMatch) {
      // No command → agent finished
      setAgentRunning(false);
      return;
    }

    // 見 heredocGuard.ts：缺結束標記的 heredoc 會讓 shell 停在 heredoc>，
    // 指令永遠不結束，這個迴圈就一直等 onExecuteCommand 的完成回呼。
    const cmd = repairUnterminatedHeredocs(cmdMatch[1].trim());
    setAgentPhase("running");
    setStuckPromptVisible(false);
    snoozeUntilRef.current = 0;

    // Execute and wait for completion
    await new Promise<void>((resolve) => {
      // 指令跑著卻完全沒有輸出多久，就提示使用者可能卡住了（見 STUCK_IDLE_MS
      // 的說明）。這個 interval 只在等這個指令跑完的期間存在——resolve
      // （指令完成）或元件卸載時務必清掉，見下面的 finish() 與掛載 effect。
      if (getIdleMs) {
        stuckIntervalRef.current = setInterval(() => {
          if (Date.now() < snoozeUntilRef.current) return;
          if (getIdleMs() >= STUCK_IDLE_MS) setStuckPromptVisible(true);
        }, STUCK_CHECK_INTERVAL_MS);
      }

      const finish = () => {
        if (stuckIntervalRef.current !== null) {
          clearInterval(stuckIntervalRef.current);
          stuckIntervalRef.current = null;
        }
        setStuckPromptVisible(false);
        resolve();
      };

      onExecuteCommand(cmd, async (block) => {
        if (agentAbortRef.current) { setAgentRunning(false); finish(); return; }

        const rawOutput = await getPtyRecentOutput(sessionId).catch(() => null)
          ?? block.rawOutput
          ?? "";
        const output = rawOutput.slice(-2000);

        const resultContent =
          `Command \`${cmd}\` finished (exit code ${block.exitCode ?? 0}).\nOutput:\n\`\`\`\n${output}\n\`\`\`\n\nContinue analyzing. If the goal has been achieved, give your final explanation (do not include any more <cmd> tags).`;

        const newHistory = [
          ...history,
          { role: "assistant" as const, content: reply },
          { role: "user" as const, content: resultContent },
        ];

        finish();
        void runAgentLoopRef.current(newHistory, systemPrompt, step + 1);
      });
    });
  }, [chat, onExecuteCommand, sessionId, locale, sendRemoteResponse, maxAgentSteps, getIdleMs]);

  useEffect(() => {
    runAgentLoopRef.current = runAgentLoop;
  }, [runAgentLoop]);

  const submitAgent = useCallback(async (text: string) => {
    setAgentRunning(true);
    setAgentStep(0);
    agentAbortRef.current = false;

    // Carry forward the prior conversation (e.g. a plan the AI already
    // proposed and is waiting on the user to confirm) — read chat.messages
    // BEFORE appending this turn's message so it reflects only prior turns.
    const priorHistory = chat.messages
      .filter((m): m is typeof m & { role: "user" | "assistant" } => m.role === "user" || m.role === "assistant")
      .map((m) => ({ role: m.role, content: contentToDisplayString(m.content) }));

    chat.addMessage({ role: "user", content: text });

    let systemPrompt: string;
    try {
      systemPrompt = await buildAgentSystemPrompt();
    } catch {
      setAgentRunning(false);
      return;
    }

    const history = [...priorHistory, { role: "user" as const, content: text }];
    await runAgentLoop(history, systemPrompt, 0);
  }, [chat, buildAgentSystemPrompt, runAgentLoop]);

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

  // MCP 是否真的會被用到。送出時與模式說明列共用同一個判斷——拆成兩份寫的話
  // 遲早會有一邊漏改，畫面就會說謊。
  const mcpActive = useMcp && mcpEnabled && mcpToolCount > 0;
  const mode: PanelMode = agentMode ? "agent" : mcpActive ? "mcp" : "suggest";
  const isDisabled = chat.isStreaming || agentRunning;

  return (
    <ChatPanelShell
      isOpen={isOpen}
      onClose={onClose}
      messages={chat.messages}
      streamBuf={chat.streamBuf}
      isStreaming={chat.isStreaming || (agentRunning && agentPhase === "thinking")}
      // **兩個階段都要有指示**：等 AI 想、以及等指令跑完——後者原本對話框
      // 是全靜的，使用者只看到氣泡消失然後乾等，回報成「空檔很長」。
      thinkingLabel={
        agentRunning
          ? agentPhase === "thinking"
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
      onSend={(text) => {
        const currentAttachments = attachments;
        setAttachments([]);
        void chat.send(text, mcpActive, undefined, currentAttachments.length > 0 ? currentAttachments : undefined);
      }}
      onSubmitAgent={(text) => {
        setAttachments([]);
        void submitAgent(text);
      }}
      mode={mode}
      maxAgentSteps={maxAgentSteps}
      mcpToolCount={mcpToolCount}
      agentRunning={agentRunning}
      agentPhase={agentPhase}
      agentStep={agentStep}
      onAbortAgent={() => {
        agentAbortRef.current = true;
        // Send Ctrl+C to PTY so a stuck command (e.g. pipe dquote>) gets
        // interrupted, the prompt reappears, and the onComplete callback
        // can fire to actually unblock the agent loop.
        writePty(sessionId, "\x03").catch(() => {});
      }}
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
          title="附加檔案"
          disabled={isDisabled}
        >
          📎
        </button>
      }
      extraInputControls={mcpEnabled ? (
        <button
          type="button"
          // Agent 迴圈是 use_mcp=false 寫死的（見 runAgentLoop），MCP 在
          // Agent 模式下不會生效——按鈕就不該繼續亮著說自己開啟。
          className={`aiterm-mcp-toggle${useMcp && mcpToolCount > 0 && !agentMode ? " aiterm-mcp-toggle--on" : ""}`}
          title={
            agentMode
              ? "Agent 模式下不使用 MCP 工具（AI 只透過終端機指令操作）"
              : mcpToolCount === 0
                ? t.mcp_toggle_no_servers
                : (useMcp ? "MCP 開啟" : "MCP 關閉")
          }
          disabled={agentMode || mcpToolCount === 0 || isDisabled}
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
