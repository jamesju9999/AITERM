import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { TerminalBlock } from "../../hooks/useTerminalBlocks";

// Mock Tauri before importing AiPanel (which imports useMcpChat).
const DEFAULT_CONFIG = {
  default_provider: null, providers: [], execution_mode: "graded",
  submit_shortcut: "enter", onboarding_done: true, max_agent_steps: 0,
  default_tab: "terminal", enterprise_server_url: null, enterprise_device_id: null, enterprise_policy: null,
};

// Per-command mock registry: tests can push response objects.
const aiChatQueue: { content: string | null; tool_calls?: unknown[]; tool_calling_unsupported?: boolean; tool_fallback_reason?: string }[] = [];
// 每次 ai_chat 帶的 toolScope（⚡／MCP 決定的工具範圍），依呼叫順序。
const aiChatScopes: (string | undefined)[] = [];
// 寫進 PTY 的資料（停止鍵沒有 onInterruptCommand 時退回送 Ctrl+C）。
const ptyWrites: string[] = [];
// Tests can push an error here to make the Nth "ai_chat" call (by call order,
// interleaved with aiChatQueue) reject instead of resolve. Keyed by the 1-based
// call index at which it should fire.
const aiChatRejectAt = new Map<number, unknown>();
// 讓某次 ai_chat 掛著不回來，用來觀察「正在等 AI」那個瞬間的畫面。
const aiChatHold: { value: Promise<never> | null } = { value: null };
// Records the `messages` array sent on each "ai_chat" invoke call, in order.
const aiChatCalls: { role: string; content: unknown }[][] = [];
// MCP 工具清單：預設空的（面板會顯示 "MCP OFF" 且按鈕停用），要測 MCP 按鈕
// 的測試自己 push 幾個進來。
const mcpTools: { name: string }[] = [];
// 面板讀了幾次設定 — 用來驗「每次開啟都重讀」。
const configCalls = { n: 0 };

const listenMock = vi.fn().mockResolvedValue(() => {});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, payload?: { messages?: { role: string; content: unknown }[]; toolScope?: string; data?: string }) => {
    if (cmd === "get_config") { configCalls.n++; return Promise.resolve(DEFAULT_CONFIG); }
    if (cmd === "get_mcp_tools") return Promise.resolve(mcpTools);
    if (cmd === "pty_write") { ptyWrites.push(payload?.data ?? ""); return Promise.resolve(); }
    if (cmd === "terminal_command_gate") return Promise.resolve({ allowed: true, reason: null });
    if (cmd === "ai_chat") {
      aiChatCalls.push(payload?.messages ?? []);
      aiChatScopes.push(payload?.toolScope);
      if (aiChatHold.value) {
        const held = aiChatHold.value;
        aiChatHold.value = null;
        return held;
      }
      const callIndex = aiChatCalls.length;
      if (aiChatRejectAt.has(callIndex)) return Promise.reject(aiChatRejectAt.get(callIndex));
      const next = aiChatQueue.shift();
      if (next) return Promise.resolve({ tool_calls: [], tool_calling_unsupported: false, ...next });
      return Promise.resolve({ content: "", tool_calls: [], tool_calling_unsupported: false });
    }
    return Promise.resolve(null);
  },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));
// 用**真實**的 translations，不要手寫只有幾個 key 的替身。
// 原本這裡是 `t: { mcp_toggle_on, mcp_toggle_off, mcp_toggle_no_servers }`，
// 於是任何新加的字串在這個測試檔裡都是 undefined——元件渲染成空白，而測試
// 看起來還是綠的。加「Agent 思考中/執行指令中」時就是這樣踩到的：斷言查不到
// 文案，卻找了半天以為是元件寫錯。
const localeState = vi.hoisted(() => ({ current: "zh-TW" as "zh-TW" | "en" }));
vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>(
    "../../lib/i18n",
  );
  return {
    // 預設繁中；個別測試可把 localeState.current 改成 "en"（測完要改回來）。
    useLocale: () => ({
      locale: localeState.current,
      t: translations[localeState.current],
      setLocale: () => {},
    }),
  };
});

// jsdom 沒有 canvas：非 jpeg/png/gif/webp 圖片的轉檔由各測試指定結果。
vi.mock("../../lib/imageToPng", () => ({ convertToPng: vi.fn() }));
import { convertToPng } from "../../lib/imageToPng";
import { AiPanel } from "./index";

// 模型呼叫內建終端機工具 run_command 的回覆。
const runCmdCall = (command: string) => ({
  content: null,
  tool_calls: [{ id: `c-${command}`, tool_name: "aiterm_terminal__run_command", server_id: "aiterm-terminal", args: { command } }],
});

beforeEach(() => {
  // ⚡／MCP 開關存在 localStorage，不清的話測試之間會互相汙染。
  localStorage.clear();
  aiChatHold.value = null;
  aiChatQueue.length = 0;
  aiChatRejectAt.clear();
  aiChatCalls.length = 0;
  aiChatScopes.length = 0;
  ptyWrites.length = 0;
  mcpTools.length = 0;
  configCalls.n = 0;
  listenMock.mockClear();
  listenMock.mockResolvedValue(() => {});
});

describe("AiPanel", () => {
  it("hides the panel via CSS class when isOpen=false", () => {
    const { container } = render(
      <AiPanel
        sessionId="s1"
        isOpen={false}
        providerName="Ollama (llama3)"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    // Panel element exists but has the hidden class — content stays mounted
    // so the chat hook keeps its listener alive while the user toggles.
    const panel = container.querySelector(".aiterm-ai-panel");
    expect(panel).not.toBeNull();
    expect(panel).toHaveClass("aiterm-ai-panel-hidden");
    // Textarea exists in DOM (not unmounted).
    // aria-hidden="true" on the panel root means Testing Library excludes it
    // from the accessible tree — use { hidden: true } to find it anyway.
    expect(screen.getByRole("textbox", { hidden: true })).toBeInTheDocument();
  });

  // The panel's glass background is only legible when backdrop-filter blurs the
  // terminal behind it. That works on macOS but not on Windows, where the
  // unblurred terminal text showed through and made the chat hard to read.
  it("keeps the translucent glass panel on non-Windows platforms", () => {
    const { container } = render(
      <AiPanel
        sessionId="s1"
        isOpen
        providerName="Ollama (llama3)"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    expect(container.querySelector(".aiterm-ai-panel")).not.toHaveClass("aiterm-ai-panel--solid");
  });

  it("switches to an opaque panel on Windows", async () => {
    const original = Object.getOwnPropertyDescriptor(navigator, "platform");
    Object.defineProperty(navigator, "platform", { value: "Win32", configurable: true });
    vi.resetModules();
    try {
      // Re-import so the module-level platform check re-evaluates as Windows.
      const { AiPanel: WinAiPanel } = await import("./index");
      const { container } = render(
        <WinAiPanel
          sessionId="s1"
          isOpen
          providerName="Ollama (llama3)"
          onClose={vi.fn()}
          onExecuteCommand={vi.fn()}
          onOpenProviderPalette={vi.fn()}
        />,
      );
      expect(container.querySelector(".aiterm-ai-panel")).toHaveClass("aiterm-ai-panel--solid");
    } finally {
      if (original) Object.defineProperty(navigator, "platform", original);
    }
  });

  it("autofocuses the textarea when transitioning to open", () => {
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox");
    expect(textbox).toHaveFocus();
  });

  it("calls onClose when Escape pressed", async () => {
    const onClose = vi.fn();
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={onClose}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("sends a message when Enter pressed", async () => {
    aiChatQueue.push({ content: "好的" });
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("好的")).toBeInTheDocument());
  });

  it("🗑 New Chat button clears messages", async () => {
    aiChatQueue.push({ content: "ok" });
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "hi");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("ok")).toBeInTheDocument());

    await userEvent.click(screen.getByRole("button", { name: /New Chat/ }));
    expect(screen.queryByText("ok")).toBeNull();
  });

  it("Agent Mode carries the prior conversation into the next AI call", async () => {
    aiChatQueue.push({ content: "這是計畫內容，要執行嗎？" });
    aiChatQueue.push({ content: "好的，已完成" });

    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );

    // Enable Agent Mode.
    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));

    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;

    // First turn: ask the AI to propose a plan (it replies with no <cmd> tag,
    // i.e. it's done for this turn and waiting on the user).
    await userEvent.type(textbox, "請規劃整理資料夾的計畫");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("這是計畫內容，要執行嗎？")).toBeInTheDocument());

    // Second turn: ask it to execute the plan just proposed.
    await userEvent.type(textbox, "請執行計畫");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("好的，已完成")).toBeInTheDocument());

    // The second ai_chat call's message history must include the first
    // turn's user request and the AI's proposed plan, not just this turn's
    // new message.
    expect(aiChatCalls.length).toBe(2);
    const secondCallContents = aiChatCalls[1].map((m) => m.content);
    expect(secondCallContents).toContain("請規劃整理資料夾的計畫");
    expect(secondCallContents).toContain("這是計畫內容，要執行嗎？");
    expect(secondCallContents).toContain("請執行計畫");
  });

  it("在 ssh 進去的遠端工作階段裡，Agent 的系統提示詞會告知別再重新連線", async () => {
    aiChatQueue.push({ content: "好的" });
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
        getRemoteSession={() => "jamesju@192.168.1.80"}
      />,
    );
    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    await userEvent.type(screen.getByRole("textbox"), "顯示 IP");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("好的")).toBeInTheDocument());

    const system = aiChatCalls[aiChatCalls.length - 1].find((m) => m.role === "system");
    expect(String(system?.content)).toContain("jamesju@192.168.1.80");
    expect(String(system?.content)).toMatch(/Do NOT run ssh/);
  });

  it("⚡ 執行 run_command 後把結果交回 AI 再問一次", async () => {
    aiChatQueue.push(runCmdCall("ls"));
    aiChatQueue.push({ content: "完成了" });

    const onExecuteCommand = vi.fn(
      (_cmd: string, onComplete?: (block: TerminalBlock) => void) => {
        onComplete?.({ id: "b1", command: "ls", rawOutput: "file.txt", status: "completed", exitCode: 0, startTime: Date.now() });
      },
    );

    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={onExecuteCommand}
        onOpenProviderPalette={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));

    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");

    // The recursive call only fires after onExecuteCommand's onComplete runs
    // (synchronously here) and the loop re-invokes itself via the ref.
    await waitFor(() => expect(screen.getByText("完成了")).toBeInTheDocument());

    expect(onExecuteCommand).toHaveBeenCalledWith("ls", expect.any(Function));
    expect(aiChatCalls.length).toBe(2);
    const secondCallContents = aiChatCalls[1].map((m) => m.content);
    expect(secondCallContents.some((c) => typeof c === "string" && c.includes("ls"))).toBe(true);
  });

  /**
   * Agent 迴圈有兩個等待階段：等 AI 想下一步、等指令跑完。狀態列原本兩段
   * 顯示同一句「Agent 執行中…」，使用者看不出來卡在哪——尤其等 AI 那段
   * 完全沒有畫面變化（實測回報）。
   */
  it("Agent 狀態列要區分「思考中」與「執行指令中」", async () => {
    aiChatQueue.push(runCmdCall("ls"));

    // 不呼叫 onComplete：讓迴圈停在「等指令跑完」那個階段，好斷言當下的文案。
    const onExecuteCommand = vi.fn();

    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={onExecuteCommand}
        onOpenProviderPalette={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");

    // 指令已送去執行、還沒回來——這時該顯示「執行指令中」而不是「思考中」。
    await waitFor(() => expect(onExecuteCommand).toHaveBeenCalled());
    // 文案與步驟數是相鄰的兩個文字節點，用 textContent 比對整塊比較穩。
    await waitFor(() => {
      const bar = document.querySelector(".aiterm-agent-status");
      expect(bar?.textContent ?? "").toContain("Agent 執行指令中");
    });
    const bar = document.querySelector(".aiterm-agent-status");
    expect(bar?.textContent ?? "").not.toContain("Agent 思考中");
    // 執行指令那段對話框**也要有指示**，只是文案不同。原本這裡是全靜的，
    // 使用者看到氣泡消失後乾等下一則訊息，回報成「空檔很長」。
    const bubble = document.querySelector(".aiterm-thinking");
    expect(bubble, "執行指令時對話框不可以完全沒有指示").not.toBeNull();
    expect(bubble?.textContent ?? "").toContain("Agent 執行指令中");
  });

  /**
   * Agent 等 AI 想下一步時，**對話框裡**要有思考氣泡。
   *
   * Agent 迴圈走自己的 invokeAiChat，不經過 chat.isStreaming，所以只看那個
   * 旗標的話 Agent 模式下對話框是全白的——使用者以為沒在運作（實測回報）。
   */
  it("Agent 等 AI 回覆時，對話框要出現思考氣泡", async () => {
    // 佇列刻意留空：mock 會回一個空 content 的 promise，但那是同步 resolve 的，
    // 停不住。改用 aiChatHold 讓這一次呼叫掛著，才觀察得到「思考中」那一刻。
    aiChatHold.value = new Promise(() => {});

    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");

    await waitFor(() => {
      expect(document.querySelector(".aiterm-thinking")).not.toBeNull();
    });
  });

  /**
   * Agent 迴圈跟一般對話走同一個 sessionId，後端的 `ai-stream` 事件本來就一直
   * 在送，`useMcpChat` 也一直在收——但 MessageList 的 isStreaming 只綁
   * chat.isStreaming，Agent 不經過它，所以字進來了卻沒人畫，使用者看到的是
   * 「想很久然後整段一次跳出來」（實測回報）。
   */
  it("Agent 等 AI 回覆時，串流的字要即時出現在對話框", async () => {
    aiChatHold.value = new Promise(() => {});

    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Ollama"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(document.querySelector(".aiterm-thinking")).not.toBeNull());

    const streamCall = listenMock.mock.calls.find((c) => c[0] === "ai-stream");
    expect(streamCall, "useMcpChat 應該有註冊 ai-stream 監聽").toBeDefined();
    const emit = streamCall![1] as (e: { payload: unknown }) => void;
    act(() => {
      emit({ payload: { session_id: "s1", kind: "chat", delta: "我先確認目錄內容", done: false } });
    });

    await waitFor(() => expect(screen.getByText("我先確認目錄內容")).toBeInTheDocument());
  });

  it("Agent Mode surfaces the error and stops when the follow-up AI call fails", async () => {
    aiChatQueue.push(runCmdCall("ls"));
    // Tauri IPC rejects with the plain AiError object the Rust side returned
    // (not a JS Error) — e.g. an expired/rotated Codex OAuth token.
    aiChatRejectAt.set(2, { kind: "auth_failed" });

    const onExecuteCommand = vi.fn(
      (_cmd: string, onComplete?: (block: TerminalBlock) => void) => {
        onComplete?.({ id: "b1", command: "ls", rawOutput: "file.txt", status: "completed", exitCode: 0, startTime: Date.now() });
      },
    );

    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Codex"
        onClose={vi.fn()}
        onExecuteCommand={onExecuteCommand}
        onOpenProviderPalette={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));

    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");

    // Command executes fine (visible in the terminal), but the follow-up
    // AI call that should analyze the result fails — the error must be
    // shown to the user (formatted via formatAiError), not silently
    // dropped or rendered as "[object Object]".
    await waitFor(() => expect(screen.getByText(/API Key 驗證失敗/)).toBeInTheDocument());
    expect(screen.queryByText(/\[object Object\]/)).not.toBeInTheDocument();
    expect(aiChatCalls.length).toBe(2);
  });

  /**
   * 有些憑證無法使用原生工具呼叫（實測：Claude Pro 訂閱的 OAuth token 只要帶
   * tools 就被算到 API credits，餘額 0 就 400）。後端會自動降級成「把工具描述
   * 注入系統提示」的文字協定，工具照樣能跑——但這不能靜默發生，使用者有權知道
   * 自己正在用比較弱的協定。
   */
  it("切換方案時要告訴使用者", async () => {
    mcpTools.push({ name: "read_file" }, { name: "write_file" });
    aiChatQueue.push({ content: "好的", tool_calling_unsupported: true, tool_fallback_reason: "unsupported" });

    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Sonnet-4.5"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");

    await waitFor(() => expect(screen.getByText("好的")).toBeInTheDocument());
    expect(screen.getByText(/切換方案/)).toBeInTheDocument();
  });


  /**
   * 「此供應商無法使用原生工具呼叫」對 Claude 訂閱是**錯的**——Claude 做得到，
   * 做不到的是「用這張訂閱憑證做工具呼叫而不被收 API 費用」。使用者看到那句話
   * 的合理反應是「明明可以，是不是壞了」，而且它也沒說這件事其實可以解決。
   */
  it("訂閱計費造成的切換要講計費，不要說模型做不到", async () => {
    mcpTools.push({ name: "read_file" });
    aiChatQueue.push({
      content: "好的",
      tool_calling_unsupported: true,
      tool_fallback_reason: "subscription_billing",
    });

    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Sonnet-4.5"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("好的")).toBeInTheDocument());

    const notice = screen.getByText(/切換方案/).textContent ?? "";
    expect(notice).toMatch(/計費|credits/);
    expect(notice).toMatch(/claude\.ai\/settings\/usage/);
    expect(notice, "不可以說模型做不到——Claude 做得到").not.toMatch(/無法使用原生工具呼叫|不支援原生工具呼叫/);
    // 不要用聽起來像次級品或在評價使用者處境的字眼。
    expect(notice, "「相容模式」暗示次級品，「準確度略低」像在評價").not.toMatch(/相容模式|準確度較低|準確度略低/);
  });

  it("沒有切換方案時不顯示那個提示", async () => {
    mcpTools.push({ name: "read_file" });
    aiChatQueue.push({ content: "好的" });

    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Sonnet-4.5"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={vi.fn()}
      />,
    );

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");

    await waitFor(() => expect(screen.getByText("好的")).toBeInTheDocument());
    expect(screen.queryByText(/切換方案/)).not.toBeInTheDocument();
  });

  describe("模式說明列", () => {
    function renderPanel(isOpen = true) {
      return render(
        <AiPanel
          sessionId="s1"
          isOpen={isOpen}
          providerName="Ollama"
          onClose={vi.fn()}
          onExecuteCommand={vi.fn()}
          onOpenProviderPalette={vi.fn()}
        />,
      );
    }

    it("切換模式時說明列跟著換", async () => {
      mcpTools.push({ name: "read_file" }, { name: "write_file" });
      renderPanel();

      // 預設 ⚡ 關：純聊天
      expect(await screen.findByText(/只會建議指令/)).toBeInTheDocument();

      // 開 ⚡：MCP 預設開著且有工具 → ⚡＋MCP
      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      expect(await screen.findByText(/2 個 MCP 工具/)).toBeInTheDocument();

      // 關 MCP → 只有 ⚡
      await userEvent.click(screen.getByRole("button", { name: /MCP \(2\)/ }));
      expect(screen.getByText(/未開啟 MCP 工具/)).toBeInTheDocument();
    });

    /**
     * Agent 跑起來之後由 .aiterm-agent-status 接手（它有步驟數與中止鈕），
     * 兩條同時堆在輸入框上只是噪音。
     */
    it("Agent 執行中時讓位給狀態列", async () => {
      aiChatQueue.push(runCmdCall("ls"));
      renderPanel();

      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      expect(screen.getByText(/未開啟 MCP 工具/)).toBeInTheDocument();

      const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
      await userEvent.type(textbox, "列出檔案");
      await userEvent.keyboard("{Enter}");

      await waitFor(() => expect(document.querySelector(".aiterm-agent-status")).not.toBeNull());
      expect(screen.queryByText(/未開啟 MCP 工具/)).not.toBeInTheDocument();
    });

    /**
     * max_agent_steps 原本只在掛載時讀一次，而面板是常駐不卸載的——使用者在
     * 設定裡改了步數，這行字（跟狀態列）都還顯示舊值，要重開 app 才會更新。
     */
    it("每次開啟面板都重讀設定", async () => {
      const { rerender } = renderPanel(false);
      const before = configCalls.n;

      rerender(
        <AiPanel
          sessionId="s1"
          isOpen={true}
          providerName="Ollama"
          onClose={vi.fn()}
          onExecuteCommand={vi.fn()}
          onOpenProviderPalette={vi.fn()}
        />,
      );

      await waitFor(() => expect(configCalls.n).toBeGreaterThan(before));
    });
  });

  /**
   * MCP 只在 ⚡ 開時有作用（⚡ 關＝純聊天、不給任何工具），所以 ⚡ 關時 MCP 按鈕
   * 要停用、不亮；但不能把使用者原本的 MCP 開關改掉——⚡ 再開時要恢復。
   */
  it("⚡ 關時 MCP 按鈕停用；開 ⚡ 後可用，而且 MCP 原本的開關狀態被保留", async () => {
    mcpTools.push({ name: "read_file" }, { name: "write_file" });
    render(
      <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
        onExecuteCommand={vi.fn()} onOpenProviderPalette={vi.fn()} />,
    );

    const mcpBtn = await screen.findByRole("button", { name: /MCP \(2\)/ });
    expect(mcpBtn).toBeDisabled();
    expect(mcpBtn.getAttribute("title")).toMatch(/⚡|Agent/);
    expect(mcpBtn.className).not.toMatch(/--on/);

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    expect(mcpBtn).toBeEnabled();
    expect(mcpBtn.className).toMatch(/--on/);

    // 關掉 MCP、關 ⚡、再開 ⚡：MCP 應該還是關的
    await userEvent.click(mcpBtn);
    await userEvent.click(screen.getByTitle(/停用 Agent 模式/));
    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    expect(mcpBtn.className).not.toMatch(/--on/);
  });

  it("英文介面：面板上的滑鼠提示與輸入框提示都是英文", async () => {
    localeState.current = "en";
    try {
      mcpTools.push({ name: "read_file" });
      render(
        <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
          onExecuteCommand={vi.fn()} onOpenProviderPalette={vi.fn()} />,
      );
      const cjk = /[\u3000-\u303f\u4e00-\u9fff\uff00-\uffef]/;
      const mcpBtn = await screen.findByRole("button", { name: /MCP \(1\)/ });
      // 面板上每個滑鼠提示與輸入框提示都不該是中文（⚡ 關、⚡ 開各看一次）。
      const chineseHints = () => [
        ...Array.from(document.querySelectorAll("[title]")).map((el) => el.getAttribute("title") ?? ""),
        ...Array.from(document.querySelectorAll("[placeholder]")).map((el) => el.getAttribute("placeholder") ?? ""),
      ].filter((s) => cjk.test(s));
      expect(chineseHints()).toEqual([]);
      await userEvent.click(screen.getByTitle(/Enable Agent mode/));
      expect(chineseHints()).toEqual([]);
      expect(mcpBtn.getAttribute("title")).toBe("MCP On");
      await userEvent.click(mcpBtn);
      expect(mcpBtn.getAttribute("title")).toBe("MCP Off");
    } finally {
      localeState.current = "zh-TW";
    }
  });

  describe("⚡／MCP 決定送給後端的工具範圍", () => {
    const send = async (text: string) => {
      await userEvent.type(screen.getByRole("textbox"), text);
      await userEvent.keyboard("{Enter}");
    };
    const mount = () => render(
      <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
        onExecuteCommand={vi.fn()} onOpenProviderPalette={vi.fn()} />,
    );

    it("⚡ 關：none", async () => {
      mcpTools.push({ name: "read_file" });
      aiChatQueue.push({ content: "好" });
      mount();
      await send("hi");
      await waitFor(() => expect(aiChatScopes).toEqual(["none"]));
    });

    it("⚡ 開、MCP 開：all", async () => {
      mcpTools.push({ name: "read_file" });
      aiChatQueue.push({ content: "好" });
      mount();
      await screen.findByRole("button", { name: /MCP \(1\)/ });
      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      await send("hi");
      await waitFor(() => expect(aiChatScopes).toEqual(["all"]));
    });

    it("⚡ 開、MCP 關：terminal", async () => {
      mcpTools.push({ name: "read_file" });
      aiChatQueue.push({ content: "好" });
      mount();
      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      await userEvent.click(await screen.findByRole("button", { name: /MCP \(1\)/ }));
      await send("hi");
      await waitFor(() => expect(aiChatScopes).toEqual(["terminal"]));
    });

    // 終端機工具就是 ⚡ 本身，不算 MCP 工具；只有它時 MCP 按鈕不該亮、⚡ 送 terminal。
    it("工具清單只有內建終端機工具時，MCP 視為沒有工具", async () => {
      mcpTools.push({ name: "aiterm_terminal__run_command" });
      aiChatQueue.push({ content: "好" });
      mount();
      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      expect(await screen.findByRole("button", { name: /MCP OFF/ })).toBeDisabled();
      await send("hi");
      await waitFor(() => expect(aiChatScopes).toEqual(["terminal"]));
    });
  });

  // 任務中途 ssh 進遠端（getRemoteSession 從 null 變成主機）：下一步的提示詞要帶上遠端說明。
  it("⚡ 每一步都重讀 getRemoteSession，中途連進遠端後下一步就帶上說明", async () => {
    let remote: string | null = null;
    aiChatQueue.push(runCmdCall("ssh jamesju@192.168.1.80"));
    aiChatQueue.push({ content: "完成了" });
    const onExecuteCommand = vi.fn((_cmd: string, onComplete?: (block: TerminalBlock) => void) => {
      remote = "jamesju@192.168.1.80";
      onComplete?.({ id: "b1", command: "ssh", rawOutput: "", status: "completed", exitCode: 0, startTime: Date.now() });
    });
    render(
      <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
        onExecuteCommand={onExecuteCommand} onOpenProviderPalette={vi.fn()}
        getRemoteSession={() => remote} />,
    );
    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
    await userEvent.type(screen.getByRole("textbox"), "連進去看 IP");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("完成了")).toBeInTheDocument());

    expect(aiChatCalls[0].some((m) => m.role === "system")).toBe(false);
    const system = aiChatCalls[1].find((m) => m.role === "system");
    expect(String(system?.content)).toContain("jamesju@192.168.1.80");
  });

  it("⚡ 關（純聊天）時不送額外的 system 提示詞，即使在遠端工作階段裡", async () => {
    aiChatQueue.push({ content: "好的" });
    render(
      <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
        onExecuteCommand={vi.fn()} onOpenProviderPalette={vi.fn()}
        getRemoteSession={() => "jamesju@192.168.1.80"} />,
    );
    await userEvent.type(screen.getByRole("textbox"), "顯示 IP");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("好的")).toBeInTheDocument());
    expect(aiChatCalls[0].some((m) => m.role === "system")).toBe(false);
  });

  describe("指令執行中按停止", () => {
    const startRunningCommand = async (onInterruptCommand?: () => void) => {
      aiChatQueue.push(runCmdCall("sleep 100"));
      const onExecuteCommand = vi.fn(); // 不呼叫完成回呼：指令一直跑著
      render(
        <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
          onExecuteCommand={onExecuteCommand} onOpenProviderPalette={vi.fn()}
          onInterruptCommand={onInterruptCommand} />,
      );
      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      await userEvent.type(screen.getByRole("textbox"), "等一下");
      await userEvent.keyboard("{Enter}");
      await waitFor(() => expect(onExecuteCommand).toHaveBeenCalled());
    };

    it("呼叫 onInterruptCommand 中斷正在跑的指令", async () => {
      const onInterruptCommand = vi.fn();
      await startRunningCommand(onInterruptCommand);
      await userEvent.click(await screen.findByTitle("停止"));
      expect(onInterruptCommand).toHaveBeenCalledTimes(1);
      expect(ptyWrites).toEqual([]);
    });

    it("沒有 onInterruptCommand 時直接送 Ctrl+C 給 PTY", async () => {
      await startRunningCommand();
      await userEvent.click(await screen.findByTitle("停止"));
      await waitFor(() => expect(ptyWrites).toEqual(["\x03"]));
    });
  });

  // 附件在 hook 遷移時被弄丟過不只一次；舊 ⚡ 迴圈也會把附件直接丟掉。兩種模式都要真的送出去。
  it.each([
    ["⚡ 關", false],
    ["⚡ 開", true],
  ])("%s 時附件會跟著訊息送給 AI", async (_label, agent) => {
    aiChatQueue.push({ content: "收到" });
    const { container } = render(
      <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
        onExecuteCommand={vi.fn()} onOpenProviderPalette={vi.fn()} />,
    );
    if (agent) await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));

    const fileInput = container.querySelector('input[type="file"]') as HTMLInputElement;
    await userEvent.upload(fileInput, new File(["附件內容"], "note.txt", { type: "text/plain" }));
    expect(await screen.findByText("note.txt")).toBeInTheDocument();

    await userEvent.type(screen.getByRole("textbox"), "看一下");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(screen.getByText("收到")).toBeInTheDocument());

    const userMsg = aiChatCalls[0].filter((m) => m.role === "user").at(-1);
    expect(JSON.stringify(userMsg?.content)).toContain("附件內容");
  });

  // 超過 5MB 的圖片以前被靜默丟掉，使用者以為附上了。
  it("超過 5MB 的圖片不附上，並在輸入框上方提示；之後成功附加就清掉提示", async () => {
    const { container } = render(
      <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
        onExecuteCommand={vi.fn()} onOpenProviderPalette={vi.fn()} />,
    );
    const fileInput = container.querySelector('input[type="file"]') as HTMLInputElement;
    const big = new File(["x"], "huge.png", { type: "image/png" });
    Object.defineProperty(big, "size", { value: 6 * 1024 * 1024 });
    await userEvent.upload(fileInput, big);

    expect(await screen.findByText("huge.png 超過 5MB，沒有附上")).toBeInTheDocument();
    expect(container.querySelector(".aiterm-attachment-pills")).toBeNull();

    await userEvent.upload(fileInput, new File(["附件內容"], "note.txt", { type: "text/plain" }));
    expect(await screen.findByText("note.txt")).toBeInTheDocument();
    expect(screen.queryByText(/超過 5MB，沒有附上/)).toBeNull();
  });

  // 各家只保證收 jpeg/png/gif/webp，其他格式先轉 PNG；轉不了或轉完太大都要告訴使用者。
  it("解不開的圖片與轉完超過 5MB 的圖片都不附上，兩種提示同時顯示", async () => {
    vi.mocked(convertToPng).mockReset();
    vi.mocked(convertToPng).mockImplementation(async (file) => {
      if ((file as File).name === "x.heic") throw new Error("decode failed");
      return `data:image/png;base64,${"A".repeat(7 * 1024 * 1024)}`; // 解碼後約 5.25MB
    });
    const { container } = render(
      <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
        onExecuteCommand={vi.fn()} onOpenProviderPalette={vi.fn()} />,
    );
    const fileInput = container.querySelector('input[type="file"]') as HTMLInputElement;
    await userEvent.upload(fileInput, [
      new File(["x"], "x.heic", { type: "image/heic" }),
      new File(["x"], "big.bmp", { type: "image/bmp" }),
    ]);

    expect(await screen.findByText("x.heic 格式無法讀取，沒有附上")).toBeInTheDocument();
    expect(screen.getByText("big.bmp 超過 5MB，沒有附上")).toBeInTheDocument();
    expect(container.querySelector(".aiterm-attachment-pills")).toBeNull();
  });

  it("轉成 PNG 的圖片照常附上並送給 AI", async () => {
    vi.mocked(convertToPng).mockReset();
    vi.mocked(convertToPng).mockResolvedValue("data:image/png;base64,UE5H");
    aiChatQueue.push({ content: "好" });
    const { container } = render(
      <AiPanel sessionId="s1" isOpen={true} providerName="Ollama" onClose={vi.fn()}
        onExecuteCommand={vi.fn()} onOpenProviderPalette={vi.fn()} />,
    );
    const fileInput = container.querySelector('input[type="file"]') as HTMLInputElement;
    await userEvent.upload(fileInput, new File(["x"], "shot.bmp", { type: "image/bmp" }));
    expect(await screen.findByText("shot.bmp")).toBeInTheDocument();
    await userEvent.type(screen.getByRole("textbox"), "看圖");
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(aiChatCalls.length).toBe(1));
    const userMsg = aiChatCalls[0].filter((m) => m.role === "user").at(-1);
    expect(JSON.stringify(userMsg?.content)).toContain("data:image/png;base64,UE5H");
  });

  /**
   * 面板預設只有 420px，長回答（尤其帶表格或程式碼的）讀起來很擠。拖寬要一路
   * 拖、而且會蓋掉終端機的寬度設定，所以給一顆「放大」直接吃滿寬。
   */
  describe("放大面板", () => {
    function renderPanel() {
      return render(
        <AiPanel
          sessionId="s1"
          isOpen={true}
          providerName="Ollama"
          onClose={vi.fn()}
          onExecuteCommand={vi.fn()}
          onOpenProviderPalette={vi.fn()}
        />,
      );
    }

    it("按下放大後面板吃滿寬", async () => {
      const { container } = renderPanel();
      const panel = container.querySelector(".aiterm-ai-panel") as HTMLElement;
      expect(panel.style.width).not.toBe("100%");

      await userEvent.click(screen.getByTitle("放大面板"));
      expect(panel.style.width).toBe("100%");
    });

    it("再按一次回到原本的寬度", async () => {
      const { container } = renderPanel();
      const panel = container.querySelector(".aiterm-ai-panel") as HTMLElement;
      const before = panel.style.width;

      await userEvent.click(screen.getByTitle("放大面板"));
      await userEvent.click(screen.getByTitle("縮小面板"));
      expect(panel.style.width).toBe(before);
    });

    // 圖示本身很細，光靠它看不出來現在是不是放大狀態（實測回報「不夠明顯」）。
    // 按鈕自己也要亮起來。
    it("展開中時按鈕要呈現啟用樣式", async () => {
      renderPanel();
      expect(screen.getByTitle("放大面板").className).not.toMatch(/--active/);

      await userEvent.click(screen.getByTitle("放大面板"));
      expect(screen.getByTitle("縮小面板").className).toMatch(/--active/);
    });

    // 滿版時左邊已經沒有終端機可以讓出來，留著拖曳手把只會讓人拖出一個
    // 「看起來沒反應」的互動。
    it("放大時收起拖曳手把", async () => {
      const { container } = renderPanel();
      expect(container.querySelector(".aiterm-panel-resize-handle")).not.toBeNull();

      await userEvent.click(screen.getByTitle("放大面板"));
      expect(container.querySelector(".aiterm-panel-resize-handle")).toBeNull();
    });
  });

  it("provider badge calls onOpenProviderPalette when clicked", async () => {
    const onOpenProviderPalette = vi.fn();
    render(
      <AiPanel
        sessionId="s1"
        isOpen={true}
        providerName="Claude"
        onClose={vi.fn()}
        onExecuteCommand={vi.fn()}
        onOpenProviderPalette={onOpenProviderPalette}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /Claude/ }));
    expect(onOpenProviderPalette).toHaveBeenCalled();
  });
});
