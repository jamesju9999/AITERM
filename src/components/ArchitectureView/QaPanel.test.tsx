import { onTestFinished } from "vitest";
import { render, screen, fireEvent, act, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../MermaidBlock", () => ({ MermaidBlock: ({ chart }: { chart: string }) => <div data-testid="mermaid">{chart}</div> }));

import { QaPanel, type QaMessage } from "./QaPanel";

const base = { busy: false, stage: "select" as const, elapsed: 0, onSend: vi.fn(), onCancel: vi.fn(), onPlayTour: vi.fn(), onExport: vi.fn(), onClear: vi.fn(), onOpenFile: vi.fn() };
const mount = async (over: Partial<Parameters<typeof QaPanel>[0]> = {}) => {
  const props = { ...base, messages: [] as QaMessage[], onSend: vi.fn(), onCancel: vi.fn(), onPlayTour: vi.fn(), onExport: vi.fn(), onClear: vi.fn(), ...over };
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

  it("沒有訊息時不顯示匯出／清除；有訊息才顯示，按匯出呼叫 onExport", async () => {
    const empty = await mount();
    expect(screen.queryByRole("button", { name: /匯出 Markdown|Export Markdown/ })).toBeNull();
    empty.unmount();
    const { props } = await mount({ messages: [{ id: "u", role: "user", text: "q" }] });
    fireEvent.click(screen.getByRole("button", { name: /匯出 Markdown|Export Markdown/ }));
    expect(props.onExport).toHaveBeenCalledTimes(1);
  });

  it("清除要按兩次；3 秒內沒按第二次就回到原狀；回答中不能清除", async () => {
    vi.useFakeTimers();
    try {
      const { props, rerender } = await mount({ messages: [{ id: "u", role: "user", text: "q" }] });
      fireEvent.click(screen.getByRole("button", { name: /^(清除記錄|Clear history)$/ }));
      expect(props.onClear).not.toHaveBeenCalled();
      act(() => { vi.advanceTimersByTime(3100); });
      expect(screen.getByRole("button", { name: /^(清除記錄|Clear history)$/ })).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: /^(清除記錄|Clear history)$/ }));
      fireEvent.click(screen.getByRole("button", { name: /再按一次確認清除|Click again to clear/ }));
      expect(props.onClear).toHaveBeenCalledTimes(1);
      rerender(<LocaleProvider><QaPanel {...props} busy messages={[{ id: "u", role: "user", text: "q" }]} /></LocaleProvider>);
      expect(screen.getByRole("button", { name: /^(清除記錄|Clear history)$/ })).toBeDisabled();
    } finally {
      vi.useRealTimers();
    }
  });

  describe("歷史", () => {
    const msgs: QaMessage[] = [
      { id: "u1", role: "user", text: "第一題登入" },
      { id: "a1", role: "ai", qa: { answer: "答一", tour: null, diagram: "", dropped: 0, truncated: false } },
      { id: "u2", role: "user", text: "第二題資料庫" },
      { id: "a2", role: "ai", qa: { answer: "答二", tour: null, diagram: "", dropped: 0, truncated: false } },
    ];
    const histBtn = () => screen.getByRole("button", { name: /^(歷史|History)/ });

    it("沒有訊息時歷史鈕仍看得到但不能按（顯示 0）；展開後列出所有問題，最新的在最上面", async () => {
      const empty = await mount();
      expect(screen.getByRole("button", { name: /^(歷史（0）|History \(0\))/ })).toBeDisabled();
      empty.unmount();
      await mount({ messages: msgs });
      expect(screen.queryByRole("list", { name: /^(歷史|History)/ })).toBeNull();
      fireEvent.click(histBtn());
      const items = within(screen.getByRole("list", { name: /^(歷史|History)/ })).getAllByRole("listitem").map((o) => o.textContent ?? "");
      expect(items[0]).toContain("第二題資料庫");
      expect(items[1]).toContain("第一題登入");
    });

    it("點問題會捲到那一題並收起清單", async () => {
      const scroll = vi.fn();
      const orig = HTMLElement.prototype.scrollIntoView;
      HTMLElement.prototype.scrollIntoView = scroll;
      onTestFinished(() => { HTMLElement.prototype.scrollIntoView = orig; });
      await mount({ messages: msgs });
      fireEvent.click(histBtn());
      fireEvent.click(screen.getByRole("button", { name: /第一題登入/ }));
      expect(scroll).toHaveBeenCalledTimes(1);
      expect((scroll.mock.contexts[0] as HTMLElement).getAttribute("data-mid")).toBe("u1");
      expect(screen.queryByRole("list", { name: /^(歷史|History)/ })).toBeNull();
    });

    it("「再問一次」把問題填進輸入框、不送出", async () => {
      const { props } = await mount({ messages: msgs });
      fireEvent.click(histBtn());
      fireEvent.click(screen.getAllByRole("button", { name: /^(再問一次|Ask again)/ })[0]);
      expect((input() as HTMLTextAreaElement).value).toBe("第二題資料庫");
      expect(props.onSend).not.toHaveBeenCalled();
      expect(screen.queryByRole("list", { name: /^(歷史|History)/ })).toBeNull();
    });

    it("Esc 收起清單", async () => {
      await mount({ messages: msgs });
      fireEvent.click(histBtn());
      fireEvent.keyDown(document, { key: "Escape" });
      expect(screen.queryByRole("list", { name: /^(歷史|History)/ })).toBeNull();
    });
  });
});
