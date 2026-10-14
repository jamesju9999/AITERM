// src/hooks/useMcpChat.test.ts
import { describe, it, expect, vi } from "vitest";

// Mock the IPC modules
vi.mock("../ipc/ai", () => ({
  aiChat: vi.fn(),
  abortAi: vi.fn(() => Promise.resolve()),
}));
vi.mock("../ipc/mcp", () => ({
  getMcpTools: vi.fn().mockResolvedValue([]),
  executeMcpTool: vi.fn(),
}));
vi.mock("../ipc/computer", () => ({
  terminalCommandGate: vi.fn(),
}));
vi.mock("../ipc/resident", () => ({
  showMainWindowAfterTask: vi.fn(() => Promise.resolve()),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { renderHook, act } from "@testing-library/react";
import { aiChat, abortAi } from "../ipc/ai";
import { executeMcpTool } from "../ipc/mcp";
import { terminalCommandGate } from "../ipc/computer";
import { showMainWindowAfterTask } from "../ipc/resident";
import { useMcpChat } from "./useMcpChat";
import { LocaleProvider } from "../contexts/LocaleContext";

describe("useMcpChat", () => {
  it("returns content when no tool calls", async () => {
    vi.mocked(aiChat).mockResolvedValueOnce({
      content: "Hello world",
      tool_calls: [],
      tool_calling_unsupported: false,
    });

    const { result } = renderHook(() => useMcpChat("session-1"), { wrapper: LocaleProvider });

    await act(async () => {
      await result.current.sendMessage("Hi", "all");
    });

    const lastMsg = result.current.messages.at(-1);
    expect(lastMsg?.role).toBe("assistant");
    // content is streamed via ai-stream events in real usage;
    // in tests the hook receives it from aiChat return value
  });

  // 模型只「宣告」要做事卻沒呼叫工具（例如「讓我先取得 UI 快照」），迴圈會就此結束。
  describe("宣告了動作卻沒呼叫工具時自動催一次", () => {
    const say = (content: string) => ({ content, tool_calls: [], tool_calling_unsupported: false });
    const call = (name: string) => ({
      content: null,
      tool_calls: [{ id: "c1", tool_name: name, server_id: "aiterm-computer", args: {} }],
      tool_calling_unsupported: false,
    });
    const mount = () => renderHook(() => useMcpChat("session-nudge"), { wrapper: LocaleProvider });

    it("催一次之後模型呼叫了工具，並繼續跑到完成", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(executeMcpTool).mockResolvedValue({ content: "tree", is_error: false });
      vi.mocked(aiChat)
        .mockResolvedValueOnce(say("讓我先取得 UI 快照。"))
        .mockResolvedValueOnce(call("aiterm_computer__computer_snapshot"))
        .mockResolvedValueOnce(say("已完成。"));
      const { result } = mount();
      await act(async () => { await result.current.sendMessage("幫我傳訊息", "all"); });

      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(3);
      expect(executeMcpTool).toHaveBeenCalledWith("aiterm_computer__computer_snapshot", {});
      // 第二次呼叫的歷史最後一則是催促（不顯示在對話裡，只送給模型）。
      const secondHistory = vi.mocked(aiChat).mock.calls[1][0];
      expect(secondHistory.at(-1)?.role).toBe("user");
      expect(String(secondHistory.at(-1)?.content)).toMatch(/直接呼叫|call the tool/i);
      expect(result.current.messages.some((m) => m.role === "user" && String(m.content).match(/直接呼叫|call the tool/i))).toBe(false);
      expect(result.current.messages.at(-1)?.content).toBe("已完成。");
    });

    it("模型一直只宣告：最多催兩次就停，不會無限迴圈", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValue(say("讓我先取得 UI 快照。"));
      const { result } = mount();
      await act(async () => { await result.current.sendMessage("幫我傳訊息", "all"); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(3);
    });

    it("模型回了空內容（沒文字也沒工具呼叫）：不顯示空白氣泡，催一次後繼續", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(executeMcpTool).mockResolvedValue({ content: "tree", is_error: false });
      vi.mocked(aiChat)
        .mockResolvedValueOnce(call("aiterm_computer__computer_snapshot"))
        .mockResolvedValueOnce(say(""))
        .mockResolvedValueOnce(call("aiterm_computer__computer_click"))
        .mockResolvedValueOnce(say("已完成。"));
      const { result } = mount();
      await act(async () => { await result.current.sendMessage("幫我點", "all"); });

      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(4);
      expect(result.current.messages.some((m) => m.role === "assistant" && String(m.content).trim() === "")).toBe(false);
      expect(result.current.messages.at(-1)?.content).toBe("已完成。");
      // 空回覆之後的歷史不能出現連續兩則 user（部分供應商會拒絕）。
      const hist = vi.mocked(aiChat).mock.calls[2][0];
      for (let i = 1; i < hist.length; i++) {
        expect(!(hist[i].role === "user" && hist[i - 1].role === "user")).toBe(true);
      }
      expect(String(hist.at(-1)?.content)).toMatch(/直接呼叫|call the tool/i);
    });

    it("一直回空內容：催兩次後顯示明確的警告，而不是空白氣泡", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValue(say(""));
      const { result } = mount();
      await act(async () => { await result.current.sendMessage("幫我點", "all"); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(3);
      const last = result.current.messages.at(-1);
      expect(last?.role).toBe("assistant");
      expect(String(last?.content).trim()).not.toBe("");
      expect(String(last?.content)).toMatch(/沒有回應|no response/i);
    });

    it("一般回答不催", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValue(say("已完成：TextEdit 現在是空的。"));
      const { result } = mount();
      await act(async () => { await result.current.sendMessage("清空", "all"); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(1);
    });

    it("沒開 MCP（沒有工具可叫）時不催", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValue(say("讓我先看看這個指令。"));
      const { result } = mount();
      await act(async () => { await result.current.sendMessage("這是什麼", "none"); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(1);
    });
  });

  // 工具呼叫上限要讀系統設定的「最大步數」（0 = 無限制），而不是寫死 10。
  describe("工具呼叫上限", () => {
    const toolCall = () => ({
      content: null,
      tool_calls: [{ id: "c", tool_name: "x__t", server_id: "x", args: {} }],
      tool_calling_unsupported: false,
    });

    it("沒指定時維持預設 10 次，超過就停並說明上限", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(executeMcpTool).mockResolvedValue({ content: "ok", is_error: false });
      vi.mocked(aiChat).mockResolvedValue(toolCall());
      const { result } = renderHook(() => useMcpChat("s-limit-default"), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("go", "all"); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(10);
      expect(String(result.current.messages.at(-1)?.content)).toContain("10");
    });

    it("使用者把最大步數設得比 10 大：照設定跑，上限訊息顯示實際數字", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(executeMcpTool).mockResolvedValue({ content: "ok", is_error: false });
      vi.mocked(aiChat).mockResolvedValue(toolCall());
      const { result } = renderHook(() => useMcpChat("s-limit-25", 25), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("go", "all"); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(25);
      expect(String(result.current.messages.at(-1)?.content)).toContain("25");
    });

    it("設得比 10 小也照設定", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(executeMcpTool).mockResolvedValue({ content: "ok", is_error: false });
      vi.mocked(aiChat).mockResolvedValue(toolCall());
      const { result } = renderHook(() => useMcpChat("s-limit-3", 3), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("go", "all"); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(3);
    });

    it("上限在送出之後才變更（設定頁剛改完）：下一次送出就吃新值，不是掛載時的舊值", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(executeMcpTool).mockResolvedValue({ content: "ok", is_error: false });
      vi.mocked(aiChat).mockResolvedValue(toolCall());
      const { result, rerender } = renderHook(({ n }) => useMcpChat("s-limit-dyn", n), {
        wrapper: LocaleProvider,
        initialProps: { n: 2 },
      });
      rerender({ n: 4 });
      await act(async () => { await result.current.sendMessage("go", "all"); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(4);
    });
  });

  // 停止鈕：中斷進行中的 AI 呼叫、不再執行後面的工具、不再繼續迴圈。
  describe("stop()", () => {
    const toolReply = (...names: string[]) => ({
      content: null,
      tool_calls: names.map((n, i) => ({ id: `c${i}`, tool_name: n, server_id: "x", args: {} })),
      tool_calling_unsupported: false,
    });
    const deferred = <T,>() => {
      let resolve!: (v: T) => void;
      let reject!: (e: unknown) => void;
      const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
      return { promise, resolve, reject };
    };
    const mount = () => renderHook(() => useMcpChat("sess-stop"), { wrapper: LocaleProvider });

    it("等 AI 回覆時按停止：呼叫後端中斷、立刻結束載入狀態、留下已停止的說明", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(abortAi).mockClear();
      const pending = deferred<never>();
      vi.mocked(aiChat).mockReturnValueOnce(pending.promise as never);
      const { result } = mount();
      let send!: Promise<void>;
      await act(async () => { send = result.current.sendMessage("幫我做事", "all"); });
      expect(result.current.isLoading).toBe(true);

      await act(async () => { result.current.stop(); });
      expect(abortAi).toHaveBeenCalledWith("sess-stop");
      expect(result.current.isLoading).toBe(false);
      expect(String(result.current.messages.at(-1)?.content)).toMatch(/已停止|stopped/i);

      // 後端被中斷後，進行中的 aiChat 會以錯誤結束：不可以變成錯誤訊息。
      await act(async () => { pending.reject({ kind: "network", message: "cancelled" }); await send; });
      expect(result.current.error).toBeNull();
    });

    it("工具執行到一半按停止：剩下的工具不再執行，也不再呼叫 AI", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(abortAi).mockClear();
      const firstTool = deferred<{ content: string; is_error: boolean }>();
      vi.mocked(executeMcpTool).mockReset();
      vi.mocked(executeMcpTool)
        .mockReturnValueOnce(firstTool.promise)
        .mockResolvedValue({ content: "should not run", is_error: false });
      vi.mocked(aiChat).mockResolvedValueOnce(toolReply("x__a", "x__b"));
      const { result } = mount();
      let send!: Promise<void>;
      await act(async () => { send = result.current.sendMessage("go", "all"); });
      await act(async () => {}); // 讓第一個工具開始執行

      await act(async () => { result.current.stop(); });
      await act(async () => { firstTool.resolve({ content: "a done", is_error: false }); await send; });

      expect(executeMcpTool).toHaveBeenCalledTimes(1); // 第二個工具沒有跑
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(1); // 沒有進下一輪
      expect(result.current.isLoading).toBe(false);
    });

    it("沒有進行中的對話時按停止：什麼都不做（不留說明、不呼叫後端）", async () => {
      vi.mocked(abortAi).mockClear();
      const { result } = mount();
      await act(async () => { result.current.stop(); });
      expect(abortAi).not.toHaveBeenCalled();
      expect(result.current.messages).toEqual([]);
    });

    it("停止之後可以馬上送下一則，且舊的那一輪不會干擾新的", async () => {
      vi.mocked(aiChat).mockReset();
      const oldCall = deferred<never>();
      vi.mocked(aiChat)
        .mockReturnValueOnce(oldCall.promise as never)
        .mockResolvedValueOnce({ content: "新的回答", tool_calls: [], tool_calling_unsupported: false });
      const { result } = mount();
      let first!: Promise<void>;
      await act(async () => { first = result.current.sendMessage("舊的", "all"); });
      await act(async () => { result.current.stop(); });
      await act(async () => { await result.current.sendMessage("新的", "all"); });
      // 舊的呼叫這時才回來（含一個工具呼叫）：必須被忽略。
      vi.mocked(executeMcpTool).mockClear();
      await act(async () => { oldCall.resolve(toolReply("x__late") as never); await first; });

      expect(executeMcpTool).not.toHaveBeenCalled();
      expect(result.current.messages.at(-1)?.content).toBe("新的回答");
      expect(result.current.isLoading).toBe(false);
    });

    it("已串流出的部分文字會保留在對話裡，再接上已停止的說明", async () => {
      vi.mocked(aiChat).mockReset();
      const pending = deferred<never>();
      vi.mocked(aiChat).mockReturnValueOnce(pending.promise as never);
      const { result } = mount();
      await act(async () => { void result.current.sendMessage("說一段話", "all"); });
      await act(async () => { result.current.stop(); });
      const last2 = result.current.messages.slice(-2).map((m) => m.role);
      expect(last2[last2.length - 1]).toBe("assistant");
    });
  });

  // 對話模式的「執行終端機指令」工具：先過閘門（執行模式×風險，必要時問使用者），再在終端機執行。
  describe("aiterm_terminal__run_command", () => {
    const TOOL = "aiterm_terminal__run_command";
    const call = (command: unknown) => ({
      content: null,
      tool_calls: [{ id: "t1", tool_name: TOOL, server_id: "aiterm-terminal", args: { command } }],
      tool_calling_unsupported: false,
    });
    const say = (content: string) => ({ content, tool_calls: [], tool_calling_unsupported: false });
    const mountWith = (runCommand?: (c: string) => Promise<string>) =>
      renderHook(() => useMcpChat("sess-term", 10, runCommand), { wrapper: LocaleProvider });

    it("允許：在終端機執行，輸出當作工具結果回給模型，並繼續下一輪", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(executeMcpTool).mockReset();
      vi.mocked(terminalCommandGate).mockReset();
      vi.mocked(terminalCommandGate).mockResolvedValue({ allowed: true, reason: null });
      const run = vi.fn().mockResolvedValue("Command `ls` finished (exit 0).\nOutput:\nfile.txt");
      vi.mocked(aiChat).mockResolvedValueOnce(call("ls")).mockResolvedValueOnce(say("看到 file.txt 了"));
      const { result } = mountWith(run);
      await act(async () => { await result.current.sendMessage("列出檔案", "all"); });

      expect(terminalCommandGate).toHaveBeenCalledWith("ls");
      expect(run).toHaveBeenCalledWith("ls");
      expect(executeMcpTool).not.toHaveBeenCalled(); // 不走後端的 MCP 呼叫
      const second = vi.mocked(aiChat).mock.calls[1][0];
      expect(String(second.at(-1)?.content)).toContain("file.txt");
      expect(result.current.messages.at(-1)?.content).toBe("看到 file.txt 了");
    });

    it("被拒絕或被擋下：不執行，原因當作錯誤結果回給模型", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(terminalCommandGate).mockReset();
      vi.mocked(terminalCommandGate).mockResolvedValue({ allowed: false, reason: "使用者拒絕執行這個指令。" });
      const run = vi.fn();
      vi.mocked(aiChat).mockResolvedValueOnce(call("rm -rf x")).mockResolvedValueOnce(say("好的，不執行"));
      const { result } = mountWith(run);
      await act(async () => { await result.current.sendMessage("刪掉", "all"); });

      expect(run).not.toHaveBeenCalled();
      const toolResult = result.current.messages.find((m) => m.role === "tool_result");
      expect(toolResult?.is_error).toBe(true);
      expect(String(toolResult?.content)).toContain("使用者拒絕");
    });

    it("指令不是字串或是空的：不過閘門、不執行，回錯誤", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(terminalCommandGate).mockReset();
      const run = vi.fn();
      vi.mocked(aiChat).mockResolvedValueOnce(call(42)).mockResolvedValueOnce(say("好"));
      const { result } = mountWith(run);
      await act(async () => { await result.current.sendMessage("x", "all"); });
      expect(terminalCommandGate).not.toHaveBeenCalled();
      expect(run).not.toHaveBeenCalled();
      expect(result.current.messages.find((m) => m.role === "tool_result")?.is_error).toBe(true);
    });

    it("沒有終端機可用（沒傳 runCommand）：回錯誤，不呼叫閘門", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(terminalCommandGate).mockReset();
      vi.mocked(aiChat).mockResolvedValueOnce(call("ls")).mockResolvedValueOnce(say("好"));
      const { result } = mountWith(undefined);
      await act(async () => { await result.current.sendMessage("x", "all"); });
      expect(terminalCommandGate).not.toHaveBeenCalled();
      expect(result.current.messages.find((m) => m.role === "tool_result")?.is_error).toBe(true);
    });

    it("閘門或執行丟出例外：變成錯誤結果，迴圈不崩潰", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(terminalCommandGate).mockReset();
      vi.mocked(terminalCommandGate).mockRejectedValue("gate down");
      vi.mocked(aiChat).mockResolvedValueOnce(call("ls")).mockResolvedValueOnce(say("好"));
      const { result } = mountWith(vi.fn());
      await act(async () => { await result.current.sendMessage("x", "all"); });
      expect(result.current.error).toBeNull();
      expect(result.current.messages.find((m) => m.role === "tool_result")?.is_error).toBe(true);
    });
  });

  describe("用過電腦操作工具，結束時把 AITerm 叫回前景", () => {
    const call = (name: string) => ({
      content: null,
      tool_calls: [{ id: "c1", tool_name: name, server_id: "x", args: {} }],
      tool_calling_unsupported: false,
    });
    const done = { content: "完成。", tool_calls: [], tool_calling_unsupported: false };

    it("呼叫過 computer 工具：結束時叫回一次", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(showMainWindowAfterTask).mockClear();
      vi.mocked(executeMcpTool).mockResolvedValue({ content: "ok", is_error: false });
      vi.mocked(aiChat).mockResolvedValueOnce(call("aiterm_computer__computer_click")).mockResolvedValueOnce(done);
      const { result } = renderHook(() => useMcpChat("session-front"), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("點一下", "all"); });
      expect(showMainWindowAfterTask).toHaveBeenCalledTimes(1);
    });

    it("只用其他工具或沒用工具：不動視窗", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(showMainWindowAfterTask).mockClear();
      vi.mocked(executeMcpTool).mockResolvedValue({ content: "ok", is_error: false });
      vi.mocked(aiChat).mockResolvedValueOnce(call("github__search")).mockResolvedValueOnce(done);
      const { result } = renderHook(() => useMcpChat("session-front2"), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("查一下", "all"); });
      vi.mocked(aiChat).mockResolvedValueOnce(done);
      await act(async () => { await result.current.sendMessage("謝謝", "all"); });
      expect(showMainWindowAfterTask).not.toHaveBeenCalled();
    });

    it("出錯結束也會叫回", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(showMainWindowAfterTask).mockClear();
      vi.mocked(executeMcpTool).mockResolvedValue({ content: "ok", is_error: false });
      vi.mocked(aiChat)
        .mockResolvedValueOnce(call("aiterm_computer__computer_type"))
        .mockRejectedValueOnce(new Error("boom"));
      const { result } = renderHook(() => useMcpChat("session-front3"), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("打字", "all"); });
      expect(showMainWindowAfterTask).toHaveBeenCalledTimes(1);
    });
  });

  describe("⚡ 合併後的工具範圍（spec 2026-10-14）", () => {
    const say = (content: string) => ({ content, tool_calls: [], tool_calling_unsupported: false });
    const runCmd = (command: string) => ({
      content: null,
      tool_calls: [{ id: "t1", tool_name: "aiterm_terminal__run_command", server_id: "aiterm-terminal", args: { command } }],
      tool_calling_unsupported: false,
    });

    it("把 scope 傳給後端（第 7 個參數）", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValueOnce(say("好"));
      const { result } = renderHook(() => useMcpChat("s-scope"), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("hi", "terminal"); });
      expect(vi.mocked(aiChat).mock.calls[0][6]).toBe("terminal");
    });

    it("額外的 system 提示詞放在送給模型的最前面，但不出現在對話裡", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValueOnce(say("好"));
      const { result } = renderHook(() => useMcpChat("s-sys"), { wrapper: LocaleProvider });
      await act(async () => {
        await result.current.sendMessage("hi", "terminal", undefined, undefined, "REMOTE-NOTE");
      });
      const sent = vi.mocked(aiChat).mock.calls[0][0];
      expect(sent[0]).toEqual({ role: "system", content: "REMOTE-NOTE" });
      expect(sent.at(-1)).toEqual({ role: "user", content: "hi" });
      expect(result.current.messages.some((m) => String(m.content).includes("REMOTE-NOTE"))).toBe(false);
    });

    it("執行工具時 phase 是 running、步數是 1，結束後 activeScope 回到 null", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(terminalCommandGate).mockResolvedValue({ allowed: true, reason: null });
      vi.mocked(aiChat).mockResolvedValueOnce(runCmd("ls")).mockResolvedValueOnce(say("完成"));
      let finishCommand: (out: string) => void = () => {};
      const runCommand = vi.fn(() => new Promise<string>((r) => { finishCommand = r; }));
      const { result } = renderHook(() => useMcpChat("s-phase", 10, runCommand), { wrapper: LocaleProvider });

      let done: Promise<void> = Promise.resolve();
      act(() => { done = result.current.sendMessage("列檔案", "terminal"); });
      await vi.waitFor(() => expect(runCommand).toHaveBeenCalledWith("ls"));
      expect(result.current.activeScope).toBe("terminal");
      expect(result.current.phase).toBe("running");
      expect(result.current.step).toBe(1);

      await act(async () => { finishCommand("ok"); await done; });
      expect(result.current.activeScope).toBeNull();
      expect(result.current.step).toBe(2);
    });

    it("純聊天（none）不催模型動手", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValueOnce(say("讓我先取得 UI 快照。"));
      const { result } = renderHook(() => useMcpChat("s-none"), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("hi", "none"); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(1);
    });
  });
});
