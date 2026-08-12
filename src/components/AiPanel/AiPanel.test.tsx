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
const aiChatQueue: { content: string; tool_calls?: unknown[]; tool_calling_unsupported?: boolean }[] = [];
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
  invoke: (cmd: string, payload?: { messages?: { role: string; content: unknown }[] }) => {
    if (cmd === "get_config") { configCalls.n++; return Promise.resolve(DEFAULT_CONFIG); }
    if (cmd === "get_mcp_tools") return Promise.resolve(mcpTools);
    if (cmd === "ai_chat") {
      aiChatCalls.push(payload?.messages ?? []);
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
vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>(
    "../../lib/i18n",
  );
  return {
    useLocale: () => ({
      locale: "zh-TW" as const,
      t: translations["zh-TW"],
      setLocale: () => {},
    }),
  };
});

import { AiPanel } from "./index";

beforeEach(() => {
  aiChatHold.value = null;
  aiChatQueue.length = 0;
  aiChatRejectAt.clear();
  aiChatCalls.length = 0;
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

  it("Agent Mode recurses to a second AI call after executing a <cmd>", async () => {
    aiChatQueue.push({ content: "<cmd>ls</cmd>" });
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
    aiChatQueue.push({ content: "<cmd>ls</cmd>" });

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
    aiChatQueue.push({ content: "<cmd>ls</cmd>" });
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
    await waitFor(() => expect(screen.getByText(/Agent 呼叫 AI 失敗，已停止/)).toBeInTheDocument());
    expect(screen.getByText(/API Key 驗證失敗/)).toBeInTheDocument();
    expect(screen.queryByText(/\[object Object\]/)).not.toBeInTheDocument();
    expect(aiChatCalls.length).toBe(2);
  });

  /**
   * 有些憑證無法使用原生工具呼叫（實測：Claude Pro 訂閱的 OAuth token 只要帶
   * tools 就被算到 API credits，餘額 0 就 400）。後端會自動降級成「把工具描述
   * 注入系統提示」的文字協定，工具照樣能跑——但這不能靜默發生，使用者有權知道
   * 自己正在用比較弱的協定。
   */
  it("降級成相容模式時要告訴使用者", async () => {
    mcpTools.push({ name: "read_file" }, { name: "write_file" });
    aiChatQueue.push({ content: "好的", tool_calling_unsupported: true });

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

    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");

    await waitFor(() => expect(screen.getByText("好的")).toBeInTheDocument());
    expect(screen.getByText(/相容模式/)).toBeInTheDocument();
  });

  it("沒有降級時不顯示那個提示", async () => {
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

    const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
    await userEvent.type(textbox, "列出檔案");
    await userEvent.keyboard("{Enter}");

    await waitFor(() => expect(screen.getByText("好的")).toBeInTheDocument());
    expect(screen.queryByText(/相容模式/)).not.toBeInTheDocument();
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

      // 預設：MCP 開著且有工具 → MCP 模式
      expect(await screen.findByText(/2 個 MCP 工具/)).toBeInTheDocument();

      // 關掉 MCP → 退回建議模式
      await userEvent.click(screen.getByRole("button", { name: /MCP \(2\)/ }));
      expect(screen.getByText(/只會建議指令/)).toBeInTheDocument();

      // 開 Agent → 換成自動執行，並明講不使用 MCP
      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      expect(screen.getByText(/不使用 MCP 工具/)).toBeInTheDocument();
    });

    /**
     * Agent 跑起來之後由 .aiterm-agent-status 接手（它有步驟數與中止鈕），
     * 兩條同時堆在輸入框上只是噪音。
     */
    it("Agent 執行中時讓位給狀態列", async () => {
      aiChatQueue.push({ content: "<cmd>ls</cmd>" });
      renderPanel();

      await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));
      expect(screen.getByText(/不使用 MCP 工具/)).toBeInTheDocument();

      const textbox = screen.getByRole("textbox") as HTMLTextAreaElement;
      await userEvent.type(textbox, "列出檔案");
      await userEvent.keyboard("{Enter}");

      await waitFor(() => expect(document.querySelector(".aiterm-agent-status")).not.toBeNull());
      expect(screen.queryByText(/不使用 MCP 工具/)).not.toBeInTheDocument();
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
   * Agent 迴圈是 `invokeAiChat(..., use_mcp=false, ...)` 寫死的，handleSubmit 走
   * agentMode 那條時也根本不讀 useMcp——所以 Agent 開著時 MCP 完全沒有作用。
   * 但按鈕原本照樣亮綠色寫著「MCP (n)」、tooltip 還說「MCP 開啟」，看起來像
   * 兩個都生效。這裡要求 UI 誠實反映它被忽略了。
   */
  it("Agent 模式開啟時，MCP 按鈕要停用並說明原因", async () => {
    mcpTools.push({ name: "read_file" }, { name: "write_file" });

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

    const mcpBtn = await screen.findByRole("button", { name: /MCP \(2\)/ });
    expect(mcpBtn).toBeEnabled();
    expect(mcpBtn.className).toMatch(/--on/);

    await userEvent.click(screen.getByTitle(/啟用 Agent 模式/));

    expect(mcpBtn).toBeDisabled();
    expect(mcpBtn.getAttribute("title")).toMatch(/Agent/);
    // 只是停用還不夠——留著亮綠的「開啟」樣式，看起來仍然像正在生效。
    expect(mcpBtn.className).not.toMatch(/--on/);
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
