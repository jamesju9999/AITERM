import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from "react";
import { invokeAiChatCtx, formatAiError, type AiError, type ChatMessage as AiChatMessage, type RemoteCtx } from "../../ipc/ai";
import { contentToDisplayString } from "../../types/attachment";
import { languageDirective } from "../../lib/i18n";
import { useLocale } from "../../contexts/LocaleContext";
import { useRemoteAiChat } from "../../hooks/useRemoteAiChat";
import { useProviderQuota } from "../../hooks/useProviderQuota";
import type { TerminalBlock } from "../../hooks/useTerminalBlocks";
import { ChatPanelShell } from "../ChatPanel/ChatPanelShell";
import { QuotaBadge } from "../QuotaBadge";

/** 單一步驟的逾時：等指令跑完（submitCommand 的 onComplete）不能等超過這麼久，
 *  否則就當作這條連線沒有 OSC 133 shell 整合、沒辦法自動接續（見
 *  t.remote_agent_no_shell_integration）。跟 AiPanel 的本機 agent 迴圈不同，
 *  那邊沒有這個逾時——同機 PTY 幾乎不會卡過「卡住偵測」自己的 120s，這裡
 *  是分享連線，品質不受控，需要一道更硬的防線。 */
const STEP_TIMEOUT_MS = 60_000;

export interface RemoteAiPanelHandle {
  submitAgent: (goal: string) => void;
  send: (text: string) => void;
  /**
   * `reason` 給的時候，且真的有 agent 迴圈在跑，會在對話裡補一則說明訊息
   * 再停止——連線事件（resync/失去控制權/連線結束）強制中止時要用，讓
   * 使用者知道「AI 剛剛不是自己停的」，不是靜默消失。使用者自己按 Stop
   * 這一種不傳 reason，維持靜默（他自己知道為什麼）。
   */
  abort: (reason?: string) => void;
}

interface AgentHistoryMsg {
  role: "user" | "assistant";
  content: string;
}

interface Props {
  connId: string;
  buildRemoteCtx: () => RemoteCtx;
  submitCommand: (cmd: string, onComplete?: (block: TerminalBlock) => void) => void;
  isControl: boolean;
  maxSteps: number;
  providerName: string;
  providerId?: string;
  onOpenProviderPalette: () => void;
  isOpen: boolean;
  onClose: () => void;
  /** RemoteTerminalView 的共享 abortRef（unmount / 連線事件會設 true）。 */
  sharedAbortRef: React.MutableRefObject<boolean>;
}

/**
 * 觀看端的 AiPanel 移植：`ChatPanelShell` 的展示層 + `useRemoteAiChat` 的
 * 單發對話 + 這裡自己的 agent 迴圈（移植自 AiPanel/index.tsx 的
 * runAgentLoop/submitAgent，改成打 ai_chat_ctx、透過 submitCommand 執行指令）。
 */
export const RemoteAiPanel = forwardRef<RemoteAiPanelHandle, Props>(function RemoteAiPanel({
  connId,
  buildRemoteCtx,
  submitCommand,
  isControl,
  maxSteps,
  providerName,
  providerId,
  onOpenProviderPalette,
  isOpen,
  onClose,
  sharedAbortRef,
}, ref) {
  const { t, locale } = useLocale();
  const chat = useRemoteAiChat(connId, buildRemoteCtx, providerId);
  const quotaWindow = useProviderQuota(providerId);

  // 觀看端的預設用途是「自己驅動別人的機器」，自由對話是次要——跟 AiPanel
  // 預設關閉 agent 模式且存 localStorage 剛好相反，這裡刻意不做持久化。
  const [agentMode, setAgentMode] = useState(true);
  const [agentRunning, setAgentRunning] = useState(false);
  const [agentStep, setAgentStep] = useState(0);
  const [agentPhase, setAgentPhase] = useState<"thinking" | "running">("thinking");
  const agentAbortRef = useRef(false);

  const isAborted = useCallback(
    () => agentAbortRef.current || sharedAbortRef.current,
    [sharedAbortRef],
  );

  const buildAgentSystemPrompt = useCallback((): string => {
    return `You are operating a REMOTE terminal over a screen-share connection. You do NOT know the current working directory or the directory contents — you have no direct filesystem access. If you need that information, run <cmd>pwd</cmd> or <cmd>ls</cmd> first before doing anything else.

You can execute shell commands via <cmd>...</cmd> tags, and iterate based on the results to accomplish the user's goal.

Rules:
1. When you need to run a command, use <cmd>shell command</cmd> (only one at a time).
2. The system will execute it automatically and return the result — keep analyzing until the goal is achieved.
3. Once the goal is achieved, give your final explanation in ${languageDirective(locale)}, and do not include any more <cmd> tags.
4. Never perform destructive or irreversible operations (e.g. rm -rf /).
5. Write all explanations in ${languageDirective(locale)}.`;
  }, [locale]);

  // Component 卸載時把本地 abort 旗標設成 true——不能只靠 sharedAbortRef
  // 由外部（RemoteTerminalView）在對的時機設定，這裡本來就該對自己的生命
  // 週期負責，不該依賴尚未寫的呼叫端程式碼。
  useEffect(() => {
    return () => {
      agentAbortRef.current = true;
    };
  }, []);

  const runAgentLoopRef = useRef<
    (history: AgentHistoryMsg[], systemPrompt: string, step: number) => Promise<void>
  >(async () => {});

  const runAgentLoop = useCallback(async (
    history: AgentHistoryMsg[],
    systemPrompt: string,
    step: number,
  ): Promise<void> => {
    if (isAborted() || step >= maxSteps) {
      if (!isAborted()) {
        chat.addMessage({
          role: "assistant",
          content: t.term_agent_max_steps(maxSteps),
        });
      }
      setAgentRunning(false);
      return;
    }

    setAgentStep(step + 1);
    setAgentPhase("thinking");

    let reply: string;
    try {
      const agentMessages: AiChatMessage[] = [
        { role: "system", content: systemPrompt },
        ...history,
      ];
      const replyObj = await invokeAiChatCtx(agentMessages, buildRemoteCtx(), connId, providerId, locale);
      reply = replyObj.content ?? "";
    } catch (e) {
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

    if (isAborted()) { setAgentRunning(false); return; }

    chat.addMessage({ role: "assistant", content: reply });

    const cmdMatch = reply.match(/<cmd>([\s\S]*?)<\/cmd>/i);
    if (!cmdMatch) {
      setAgentRunning(false);
      return;
    }

    if (isAborted()) { setAgentRunning(false); return; }

    const cmd = cmdMatch[1].trim();
    setAgentPhase("running");

    await new Promise<void>((resolve) => {
      let settled = false;
      let timeoutId: ReturnType<typeof setTimeout> | null = null;

      const finish = () => {
        if (settled) return;
        settled = true;
        if (timeoutId !== null) {
          clearTimeout(timeoutId);
          timeoutId = null;
        }
        resolve();
      };

      timeoutId = setTimeout(() => {
        if (settled) return;
        if (isAborted()) {
          // 使用者已經按過停止（或 sharedAbortRef 被外部設成 true）——
          // 不是這條連線沒有 shell 整合，不該用那句話蓋過使用者自己的
          // 停止動作。agentRunning 在 abort() 當下已經設成 false 了。
          finish();
          return;
        }
        chat.addMessage({
          role: "assistant",
          content: t.remote_agent_no_shell_integration,
        });
        setAgentRunning(false);
        finish();
      }, STEP_TIMEOUT_MS);

      submitCommand(cmd, (block) => {
        if (settled) return; // timeout already fired and ended the loop
        if (isAborted()) { setAgentRunning(false); finish(); return; }

        const output = (block.rawOutput ?? "").slice(-2000);
        const resultContent =
          `Command \`${cmd}\` finished (exit code ${block.exitCode ?? 0}).\nOutput:\n\`\`\`\n${output}\n\`\`\`\n\nContinue analyzing. If the goal has been achieved, give your final explanation (do not include any more <cmd> tags).`;

        const newHistory: AgentHistoryMsg[] = [
          ...history,
          { role: "assistant", content: reply },
          { role: "user", content: resultContent },
        ];

        finish();
        void runAgentLoopRef.current(newHistory, systemPrompt, step + 1);
      });
    });
  }, [chat, submitCommand, buildRemoteCtx, connId, providerId, locale, maxSteps, t, isAborted]);

  useEffect(() => {
    runAgentLoopRef.current = runAgentLoop;
  }, [runAgentLoop]);

  const submitAgent = useCallback((text: string) => {
    if (!isControl) return;
    setAgentRunning(true);
    setAgentStep(0);
    agentAbortRef.current = false;
    // 一次過去的 resync/失去控制權會把 sharedAbortRef 釘死在 true，不會有
    // 任何地方把它改回 false（那是 RemoteTerminalView 的責任範圍之外——它
    // 只負責「設 true」，不負責重置）。不重置的話，一次連線小插曲就會讓
    // 這個分頁之後每一次 /agent、/ai 都在 runAgentLoop 第一行被判定成
    // 已中止，安靜地 no-op，不會有任何錯誤訊息。
    // 這裡可以放心重置：isControl 是 RemoteTerminalView 當下的即時 phase
    // 判斷（"live" 且 "control"），能走到這行代表連線現在確實在控制模式
    // 下——不管 sharedAbortRef 是被哪一次過去的 resync 或失去控制權設成
    // true，那次事件描述的狀態都已經不是現在了，重置是安全的。
    sharedAbortRef.current = false;

    const priorHistory: AgentHistoryMsg[] = chat.messages
      .filter((m): m is typeof m & { role: "user" | "assistant" } => m.role === "user" || m.role === "assistant")
      .map((m) => ({ role: m.role, content: contentToDisplayString(m.content) }));

    chat.addMessage({ role: "user", content: text });

    const systemPrompt = buildAgentSystemPrompt();
    const history: AgentHistoryMsg[] = [...priorHistory, { role: "user", content: text }];
    void runAgentLoop(history, systemPrompt, 0);
  }, [isControl, chat, buildAgentSystemPrompt, runAgentLoop, sharedAbortRef]);

  const send = useCallback((text: string) => {
    if (!isControl) return;
    void chat.send(text);
  }, [isControl, chat]);

  const abort = useCallback((reason?: string) => {
    agentAbortRef.current = true;
    // 只有「真的有迴圈在跑」時才補說明訊息——連線事件即使沒有 mission
    // 在飛也一律呼叫 abort()，沒有這個檢查會對著空對話塞一則沒頭沒尾的
    // 「AI 代理已停止」。讀 agentRunning 而不是等 setAgentRunning(false)
    // 生效後再讀：這個 callback 本身就會在 agentRunning 改變時被
    // useCallback 重新建立，這裡讀到的一定是呼叫當下最新的值。
    if (reason && agentRunning) {
      chat.addMessage({ role: "assistant", content: reason });
    }
    setAgentRunning(false);
  }, [chat, agentRunning]);

  useImperativeHandle(ref, () => ({ submitAgent, send, abort }), [submitAgent, send, abort]);

  return (
    <ChatPanelShell
      isOpen={isOpen}
      onClose={onClose}
      messages={chat.messages}
      streamBuf={chat.streamBuf}
      isStreaming={chat.isStreaming || (agentRunning && agentPhase === "thinking")}
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
      onExecuteCommand={(cmd) => submitCommand(cmd)}
      agentMode={agentMode}
      onToggleAgentMode={() => setAgentMode((x) => !x)}
      onSend={send}
      onSubmitAgent={submitAgent}
      mode={agentMode ? "agent" : "suggest"}
      maxAgentSteps={maxSteps}
      agentRunning={agentRunning}
      agentPhase={agentPhase}
      agentStep={agentStep}
      onAbortAgent={abort}
      providerName={providerName}
      onOpenProviderPalette={onOpenProviderPalette}
      headerBadge={quotaWindow ? <QuotaBadge window={quotaWindow} /> : undefined}
      sessions={chat.sessions}
      onLoadSession={(s) => chat.loadMessages(s.messages, s.id)}
      onNewChat={() => chat.clear()}
      onDeleteSession={(id) => chat.deleteSession(id)}
      inputDisabled={!isControl}
    />
  );
});
