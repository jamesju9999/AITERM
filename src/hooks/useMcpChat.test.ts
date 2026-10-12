// src/hooks/useMcpChat.test.ts
import { describe, it, expect, vi } from "vitest";

// Mock the IPC modules
vi.mock("../ipc/ai", () => ({
  aiChat: vi.fn(),
}));
vi.mock("../ipc/mcp", () => ({
  getMcpTools: vi.fn().mockResolvedValue([]),
  executeMcpTool: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { renderHook, act } from "@testing-library/react";
import { aiChat } from "../ipc/ai";
import { executeMcpTool } from "../ipc/mcp";
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
      await result.current.sendMessage("Hi", true);
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
      await act(async () => { await result.current.sendMessage("幫我傳訊息", true); });

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
      await act(async () => { await result.current.sendMessage("幫我傳訊息", true); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(3);
    });

    it("一般回答不催", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValue(say("已完成：TextEdit 現在是空的。"));
      const { result } = mount();
      await act(async () => { await result.current.sendMessage("清空", true); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(1);
    });

    it("沒開 MCP（沒有工具可叫）時不催", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(aiChat).mockResolvedValue(say("讓我先看看這個指令。"));
      const { result } = mount();
      await act(async () => { await result.current.sendMessage("這是什麼", false); });
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
      await act(async () => { await result.current.sendMessage("go", true); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(10);
      expect(String(result.current.messages.at(-1)?.content)).toContain("10");
    });

    it("使用者把最大步數設得比 10 大：照設定跑，上限訊息顯示實際數字", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(executeMcpTool).mockResolvedValue({ content: "ok", is_error: false });
      vi.mocked(aiChat).mockResolvedValue(toolCall());
      const { result } = renderHook(() => useMcpChat("s-limit-25", 25), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("go", true); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(25);
      expect(String(result.current.messages.at(-1)?.content)).toContain("25");
    });

    it("設得比 10 小也照設定", async () => {
      vi.mocked(aiChat).mockReset();
      vi.mocked(executeMcpTool).mockResolvedValue({ content: "ok", is_error: false });
      vi.mocked(aiChat).mockResolvedValue(toolCall());
      const { result } = renderHook(() => useMcpChat("s-limit-3", 3), { wrapper: LocaleProvider });
      await act(async () => { await result.current.sendMessage("go", true); });
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
      await act(async () => { await result.current.sendMessage("go", true); });
      expect(vi.mocked(aiChat)).toHaveBeenCalledTimes(4);
    });
  });
});
