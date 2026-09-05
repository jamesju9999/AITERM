import { renderHook, act } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const listTasks = vi.fn();
const readTranscript = vi.fn();
const setSummary = vi.fn();
const saveReport = vi.fn();
const invokeAiChat = vi.fn();

vi.mock("../../ipc/tasks", () => ({
  listTasks: (...a: unknown[]) => listTasks(...a),
  readTranscript: (...a: unknown[]) => readTranscript(...a),
  setSummary: (...a: unknown[]) => setSummary(...a),
}));
vi.mock("../../ipc/reports", () => ({ saveReport: (...a: unknown[]) => saveReport(...a) }));
vi.mock("../../ipc/ai", () => ({ invokeAiChat: (...a: unknown[]) => invokeAiChat(...a) }));

import { useWorkReport } from "./useWorkReport";

const card = (over: Record<string, unknown> = {}) => ({
  id: "t1",
  title: "卡片",
  body: "",
  project_dir: "/r",
  status: "done",
  parallel_ok: true,
  interactive: false,
  sort_order: 1,
  outcome: "success",
  tab_id: "tab-1",
  transcript_path: "/p/t.txt",
  error_message: null,
  created_at: "2026-09-05T10:00:00Z",
  dispatched_at: null,
  finished_at: null,
  ai_summary: null,
  attachments: [],
  ...over,
});

const ARTIFACT = "說明\n\n```artifact-html\n<html><title>報告</title></html>\n```";

describe("useWorkReport", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    readTranscript.mockResolvedValue("終端機輸出");
    setSummary.mockResolvedValue(undefined);
    saveReport.mockResolvedValue("2026-09-05-1430.html");
  });

  it("已經有 ai_summary 的卡片不重複呼叫 AI 做摘要", async () => {
    listTasks.mockResolvedValue([card({ ai_summary: "已經有的摘要" })]);
    invokeAiChat.mockResolvedValue({ content: ARTIFACT, tool_calls: [] });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    // 只有第二階段那一次
    expect(invokeAiChat).toHaveBeenCalledTimes(1);
    expect(readTranscript).not.toHaveBeenCalled();
  });

  it("只對已完成的卡片做摘要", async () => {
    listTasks.mockResolvedValue([
      card({ id: "a", status: "done" }),
      card({ id: "b", status: "planning", outcome: null }),
    ]);
    invokeAiChat
      .mockResolvedValueOnce({ content: "摘要 A", tool_calls: [] })
      .mockResolvedValueOnce({ content: ARTIFACT, tool_calls: [] });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    expect(setSummary).toHaveBeenCalledTimes(1);
    expect(setSummary).toHaveBeenCalledWith("p1", "a", "摘要 A");
  });

  // 十張卡因為一張失敗而全部白跑，代價太高。
  it("某張摘要失敗時繼續跑完其他張並產出報告", async () => {
    listTasks.mockResolvedValue([
      card({ id: "a", status: "done" }),
      card({ id: "b", status: "done" }),
    ]);
    invokeAiChat
      .mockRejectedValueOnce(new Error("網路錯誤"))
      .mockResolvedValueOnce({ content: "摘要 B", tool_calls: [] })
      .mockResolvedValueOnce({ content: ARTIFACT, tool_calls: [] });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    expect(setSummary).toHaveBeenCalledWith("p1", "b", "摘要 B");
    expect(saveReport).toHaveBeenCalled();
    expect(result.current.html).toContain("<title>報告</title>");
  });

  // 兩張卡而不是一張：一張的話「第一次就中止」跟「每張都跑、每張都失敗」
  // 分不出來，測試會對兩種實作都亮綠燈。
  it("AI 未設定時在第一次失敗就中止，不繼續打後面的卡片", async () => {
    listTasks.mockResolvedValue([
      card({ id: "a", status: "done" }),
      card({ id: "b", status: "done" }),
    ]);
    invokeAiChat.mockRejectedValue({ kind: "not_configured" });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    expect(invokeAiChat).toHaveBeenCalledTimes(1);
    expect(setSummary).not.toHaveBeenCalled();
    expect(saveReport).not.toHaveBeenCalled();
    expect(result.current.error).toBeTruthy();
  });

  // 默默存一份空檔的話，使用者會以為有報告、打開卻是空的。
  it("回覆裡沒有 artifact 區塊時報錯且不存檔", async () => {
    listTasks.mockResolvedValue([card({ ai_summary: "有了" })]);
    invokeAiChat.mockResolvedValue({ content: "我沒有產生文件", tool_calls: [] });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    expect(saveReport).not.toHaveBeenCalled();
    expect(result.current.error).toBeTruthy();
    expect(result.current.rawReply).toContain("我沒有產生文件");
  });

  it("專案沒有任何卡片時擋下，不呼叫 AI", async () => {
    listTasks.mockResolvedValue([]);
    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    expect(invokeAiChat).not.toHaveBeenCalled();
    expect(result.current.error).toBeTruthy();
  });

  it("第二階段必須帶 supportsArtifacts=true", async () => {
    listTasks.mockResolvedValue([card({ ai_summary: "有了" })]);
    invokeAiChat.mockResolvedValue({ content: ARTIFACT, tool_calls: [] });

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    // 第六個參數
    expect(invokeAiChat.mock.calls[0][5]).toBe(true);
  });

  it("存檔失敗時報告仍然顯示，並說明沒存成功", async () => {
    listTasks.mockResolvedValue([card({ ai_summary: "有了" })]);
    invokeAiChat.mockResolvedValue({ content: ARTIFACT, tool_calls: [] });
    saveReport.mockRejectedValue(new Error("磁碟滿了"));

    const { result } = renderHook(() => useWorkReport("p1", "我的專案"));
    await act(async () => { await result.current.generate("review"); });

    expect(result.current.html).toContain("<title>報告</title>");
    expect(result.current.error).toBeTruthy();
  });
});
