import { render, screen, fireEvent, act, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../MermaidBlock", () => ({ MermaidBlock: ({ chart }: { chart: string }) => <div data-testid="mermaid">{chart}</div> }));

import { QaPanel, type QaMessage } from "./QaPanel";

const base = { busy: false, stage: "select" as const, elapsed: 0, onSend: vi.fn(), onCancel: vi.fn(), onPlayTour: vi.fn(), onOpenFile: vi.fn() };
const mount = async (over: Partial<Parameters<typeof QaPanel>[0]> = {}) => {
  const props = { ...base, messages: [] as QaMessage[], onSend: vi.fn(), onCancel: vi.fn(), onPlayTour: vi.fn(), ...over };
  const r = render(<LocaleProvider><QaPanel {...props} /></LocaleProvider>);
  await act(async () => {});
  return { ...r, props };
};
const input = () => screen.getByRole("textbox");

describe("QaPanel", () => {
  it("空狀態顯示範例問題，點擊會填入輸入框而不送出", async () => {
    const { props } = await mount();
    fireEvent.click(screen.getByRole("button", { name: /使用者登入的流程怎麼跑|How does user login work/ }));
    expect((input() as HTMLTextAreaElement).value).toMatch(/使用者登入的流程怎麼跑|How does user login work/);
    expect(props.onSend).not.toHaveBeenCalled();
  });

  it("Enter 送出並清空；Shift+Enter 不送出；空白不送出", async () => {
    const { props } = await mount();
    fireEvent.keyDown(input(), { key: "Enter" });
    expect(props.onSend).not.toHaveBeenCalled();
    fireEvent.change(input(), { target: { value: "  hi  " } });
    fireEvent.keyDown(input(), { key: "Enter", shiftKey: true });
    expect(props.onSend).not.toHaveBeenCalled();
    fireEvent.keyDown(input(), { key: "Enter" });
    expect(props.onSend).toHaveBeenCalledWith("hi");
    expect((input() as HTMLTextAreaElement).value).toBe("");
  });

  it("輸入法組字中的 Enter 不送出", async () => {
    const { props } = await mount();
    fireEvent.change(input(), { target: { value: "你好" } });
    fireEvent.keyDown(input(), { key: "Enter", isComposing: true });
    expect(props.onSend).not.toHaveBeenCalled();
  });

  describe("輸入法確認候選字的 Enter（WKWebView：compositionend 先於 keydown）", () => {
    afterEach(() => vi.useRealTimers());

    it("compositionend 後立刻 Enter（keyCode 13、isComposing=false）不送出；150ms 後才送出", async () => {
      const { props } = await mount();
      vi.useFakeTimers();
      fireEvent.change(input(), { target: { value: "你好" } });
      fireEvent.compositionStart(input());
      fireEvent.compositionEnd(input());
      fireEvent.keyDown(input(), { key: "Enter", keyCode: 13 });
      expect(props.onSend).not.toHaveBeenCalled();
      act(() => { vi.advanceTimersByTime(150); });
      fireEvent.keyDown(input(), { key: "Enter", keyCode: 13 });
      expect(props.onSend).toHaveBeenCalledWith("你好");
    });

    it("組字中 (keyCode 229) 不送出", async () => {
      const { props } = await mount();
      fireEvent.change(input(), { target: { value: "你好" } });
      fireEvent.keyDown(input(), { key: "Enter", keyCode: 229 });
      expect(props.onSend).not.toHaveBeenCalled();
    });

    it("沒組字的 Enter 送出；Shift+Enter 不送出", async () => {
      const { props } = await mount();
      fireEvent.change(input(), { target: { value: "hi" } });
      fireEvent.keyDown(input(), { key: "Enter", keyCode: 13, shiftKey: true });
      expect(props.onSend).not.toHaveBeenCalled();
      fireEvent.keyDown(input(), { key: "Enter", keyCode: 13 });
      expect(props.onSend).toHaveBeenCalledWith("hi");
    });
  });

  it("忙碌時顯示階段、m:ss 與取消；不能送出", async () => {
    const { props } = await mount({ busy: true, stage: "answer", elapsed: 65 });
    expect(screen.getByText(/閱讀檔案並回答|Reading files and answering/)).toBeInTheDocument();
    expect(screen.getByText("1:05")).toBeInTheDocument();
    fireEvent.change(input(), { target: { value: "x" } });
    fireEvent.keyDown(input(), { key: "Enter" });
    expect(props.onSend).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /取消|Cancel/ }));
    expect(props.onCancel).toHaveBeenCalled();
  });

  it("Esc 事件不外泄", async () => {
    await mount();
    const outer = vi.fn();
    document.addEventListener("keydown", outer);
    fireEvent.keyDown(input(), { key: "Escape" });
    document.removeEventListener("keydown", outer);
    expect(outer).not.toHaveBeenCalled();
  });

  it("AI 回答以 Markdown 渲染，有導覽顯示播放鈕，有 diagram 顯示圖，dropped 顯示提示", async () => {
    const msgs: QaMessage[] = [
      { id: "u1", role: "user", text: "怎麼登入" },
      { id: "a1", role: "ai", qa: { answer: "**粗體答案**", tour: { title: "T", steps: [{ module: "a", caption: "c" }, { module: "b", caption: "d" }] }, diagram: "flowchart LR\n A-->B", dropped: 2, truncated: false } },
      { id: "a2", role: "ai", qa: { answer: "沒有導覽", tour: null, diagram: "", dropped: 0, truncated: false } },
    ];
    const { props } = await mount({ messages: msgs });
    expect(screen.getByText("怎麼登入")).toBeInTheDocument();
    expect(screen.getByText("粗體答案").tagName).toBe("STRONG");
    const play = screen.getAllByRole("button", { name: /在流程圖上播放（2 步）|Play on the diagram \(2 steps\)/ });
    expect(play).toHaveLength(1); // 第二則沒有導覽
    fireEvent.click(play[0]);
    expect(props.onPlayTour).toHaveBeenCalledWith("a1");
    expect(screen.getAllByTestId("mermaid")).toHaveLength(1);
    expect(screen.getByTestId("mermaid").textContent).toContain("A-->B");
    expect(screen.getByText(/已丟棄 2 個無法驗證的步驟|Dropped 2 steps/)).toBeInTheDocument();
    expect(screen.queryByText(/已丟棄 0|Dropped 0/)).toBeNull();
  });

  it("只有 truncated 的回答顯示輸出長度上限警告", async () => {
    const msgs: QaMessage[] = [
      { id: "a1", role: "ai", qa: { answer: "完整答案", tour: null, diagram: "", dropped: 0, truncated: false } },
      { id: "a2", role: "ai", qa: { answer: "半截答案", tour: null, diagram: "", dropped: 0, truncated: true } },
    ];
    await mount({ messages: msgs });
    expect(screen.getAllByText(/回覆可能被模型的輸出長度上限截斷|reply may have been cut off/i)).toHaveLength(1);
  });

  it("錯誤訊息顯示為錯誤泡泡", async () => {
    await mount({ messages: [{ id: "e1", role: "error", text: "AI 回應不是有效的格式" }] });
    expect(within(screen.getByRole("alert")).getByText("AI 回應不是有效的格式")).toBeInTheDocument();
  });

  it("錯誤泡泡有 raw 才出現「顯示原始回應」，點開可見原文，再點收起", async () => {
    const { rerender, props } = await mount({ messages: [{ id: "e0", role: "error", text: "沒有 raw" }] });
    expect(screen.queryByRole("button", { name: /顯示原始回應|Show raw response/ })).toBeNull();
    rerender(<LocaleProvider><QaPanel {...props} messages={[{ id: "e1", role: "error", text: "格式錯", raw: '{"answer": ' }]} /></LocaleProvider>);
    await act(async () => {});
    expect(screen.queryByTestId("qa-raw")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /顯示原始回應|Show raw response/ }));
    expect(screen.getByTestId("qa-raw").textContent).toBe('{"answer": ');
    expect(screen.queryByText(/已截斷|truncated/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /隱藏原始回應|Hide raw response/ }));
    expect(screen.queryByTestId("qa-raw")).toBeNull();
  });

  it("原始回應超過 4000 字只顯示前 4000 字並註明已截斷", async () => {
    const raw = "a".repeat(4000) + "TAIL";
    await mount({ messages: [{ id: "e1", role: "error", text: "格式錯", raw }] });
    fireEvent.click(screen.getByRole("button", { name: /顯示原始回應|Show raw response/ }));
    expect(screen.getByTestId("qa-raw").textContent).toBe("a".repeat(4000));
    expect(screen.getByText(/已截斷|truncated/)).toBeInTheDocument();
  });

  it("剛好 4000 字不算截斷", async () => {
    await mount({ messages: [{ id: "e1", role: "error", text: "格式錯", raw: "b".repeat(4000) }] });
    fireEvent.click(screen.getByRole("button", { name: /顯示原始回應|Show raw response/ }));
    expect(screen.getByTestId("qa-raw").textContent).toHaveLength(4000);
    expect(screen.queryByText(/已截斷|truncated/)).toBeNull();
  });
});
