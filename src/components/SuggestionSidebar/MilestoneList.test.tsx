import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import type { MilestoneState } from "../../lib/milestones";

const invokeAiComplete = vi.fn();
const abortAi = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/ai", () => ({
  invokeAiComplete: (...a: unknown[]) => invokeAiComplete(...a),
  abortAi: (...a: unknown[]) => abortAi(...a),
  formatAiError: () => "ERR",
}));
let screenText: string | null = "目前畫面內容 CURRENT";
vi.mock("../../lib/terminalInstanceRegistry", () => ({ serializeTerminal: () => screenText }));
vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await vi.importActual<typeof import("../../lib/i18n")>("../../lib/i18n");
  return { useLocale: () => ({ locale: "zh-TW" as const, t: translations["zh-TW"], setLocale: () => {} }) };
});

import { MilestoneList } from "./MilestoneList";
import { PLAIN_COMPLETION_SYSTEM_PROMPT } from "../../lib/promptSuggestions";

const reply = (content: string | null) => ({ content, tool_calls: [], tool_calling_unsupported: false });
const GOAL = "將舊程式的 Client-Server 架構轉換為網頁平台架構";
const S = (items: [string, string, boolean][], forGoal = GOAL): MilestoneState => ({
  forGoal,
  items: items.map(([id, text, done]) => ({ id, text, done })),
});
const THREE = S([["a", "盤點 API", true], ["b", "拆分登入模組", false], ["c", "遷移資料庫", false]]);

const onChange = vi.fn();
const onFocus = vi.fn();
const flush = () => act(async () => {});

const setup = (p: Partial<React.ComponentProps<typeof MilestoneList>> = {}) =>
  render(
    <MilestoneList sessionId="s1" goal={GOAL} state={undefined} onChange={onChange} getHistory={() => []} focusId={null} onFocus={onFocus} redact {...p} />,
  );

beforeEach(() => {
  invokeAiComplete.mockReset();
  abortAi.mockClear();
  onChange.mockClear();
  onFocus.mockClear();
  screenText = "目前畫面內容 CURRENT";
});

describe("MilestoneList – manual editing", () => {
  it("shows an empty-state hint and the action buttons", () => {
    setup();
    expect(screen.getByText(/還沒有里程碑/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "AI 拆解" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "＋ 新增" })).toBeTruthy();
  });

  it("shows progress and every item", () => {
    setup({ state: THREE });
    expect(screen.getByText("1/3")).toBeTruthy();
    expect(screen.getByText("盤點 API")).toBeTruthy();
    expect(screen.getByRole("checkbox", { name: "完成：盤點 API" })).toHaveProperty("checked", true);
    expect(screen.getByRole("checkbox", { name: "完成：拆分登入模組" })).toHaveProperty("checked", false);
  });

  it("ticking a checkbox flips only that item and keeps the goal it was planned for", () => {
    setup({ state: THREE });
    fireEvent.click(screen.getByRole("checkbox", { name: "完成：拆分登入模組" }));
    const next = onChange.mock.calls[0][0] as MilestoneState;
    expect(next.forGoal).toBe(GOAL);
    expect(next.items.map((i) => i.done)).toEqual([true, true, false]);
  });

  it("adds a milestone; with no state yet it records the current goal", () => {
    setup();
    fireEvent.click(screen.getByRole("button", { name: "＋ 新增" }));
    fireEvent.change(screen.getByRole("textbox", { name: "里程碑文字" }), { target: { value: "  新的一項  " } });
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    const next = onChange.mock.calls[0][0] as MilestoneState;
    expect(next.forGoal).toBe(GOAL);
    expect(next.items).toHaveLength(1);
    expect(next.items[0].text).toBe("新的一項");
    expect(next.items[0].done).toBe(false);
  });

  it("a manual edit never changes which goal the milestones were planned for", () => {
    setup({ state: THREE, goal: "後來改掉的新目標" });
    fireEvent.click(screen.getByRole("checkbox", { name: "完成：拆分登入模組" }));
    expect((onChange.mock.calls[0][0] as MilestoneState).forGoal).toBe(GOAL);
  });

  it("appends a new milestone after the existing ones", () => {
    setup({ state: THREE });
    fireEvent.click(screen.getByRole("button", { name: "＋ 新增" }));
    fireEvent.change(screen.getByRole("textbox", { name: "里程碑文字" }), { target: { value: "最後一項" } });
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect((onChange.mock.calls[0][0] as MilestoneState).items.map((i) => i.text)).toEqual(["盤點 API", "拆分登入模組", "遷移資料庫", "最後一項"]);
  });

  it("does not add a blank milestone", () => {
    setup();
    fireEvent.click(screen.getByRole("button", { name: "＋ 新增" }));
    fireEvent.change(screen.getByRole("textbox", { name: "里程碑文字" }), { target: { value: "   " } });
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(onChange).not.toHaveBeenCalled();
  });

  it("disables adding once the maximum is reached", () => {
    const ten = S(Array.from({ length: 10 }, (_, i) => [`i${i}`, `項目${i}`, false] as [string, string, boolean]));
    setup({ state: ten });
    expect((screen.getByRole("button", { name: "＋ 新增" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("edits an item's text, and cancel leaves it alone", () => {
    setup({ state: THREE });
    fireEvent.click(screen.getByRole("button", { name: "編輯：拆分登入模組" }));
    const box = screen.getByRole("textbox", { name: "里程碑文字" }) as HTMLInputElement;
    expect(box.value).toBe("拆分登入模組");
    fireEvent.change(box, { target: { value: "登入改成 REST" } });
    fireEvent.click(screen.getByRole("button", { name: "取消" }));
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "編輯：拆分登入模組" }));
    fireEvent.change(screen.getByRole("textbox", { name: "里程碑文字" }), { target: { value: "登入改成 REST" } });
    fireEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect((onChange.mock.calls[0][0] as MilestoneState).items[1].text).toBe("登入改成 REST");
  });

  it("deletes an item, and deleting the last one clears the state", () => {
    setup({ state: THREE });
    fireEvent.click(screen.getByRole("button", { name: "刪除：遷移資料庫" }));
    expect((onChange.mock.calls[0][0] as MilestoneState).items.map((i) => i.id)).toEqual(["a", "b"]);
    onChange.mockClear();
    setup({ state: S([["z", "唯一一項", false]]) });
    fireEvent.click(screen.getByRole("button", { name: "刪除：唯一一項" }));
    expect(onChange).toHaveBeenCalledWith(undefined);
  });

  it("moves items up and down, and the ends cannot move further", () => {
    setup({ state: THREE });
    fireEvent.click(screen.getByRole("button", { name: "上移：拆分登入模組" }));
    expect((onChange.mock.calls[0][0] as MilestoneState).items.map((i) => i.id)).toEqual(["b", "a", "c"]);
    onChange.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "下移：拆分登入模組" }));
    expect((onChange.mock.calls[0][0] as MilestoneState).items.map((i) => i.id)).toEqual(["a", "c", "b"]);
    expect((screen.getByRole("button", { name: "上移：盤點 API" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "下移：遷移資料庫" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("can be collapsed", () => {
    setup({ state: THREE });
    const head = screen.getByRole("button", { name: /里程碑/ });
    expect(head.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(head);
    expect(head.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("盤點 API")).toBeNull();
  });
});

describe("MilestoneList – AI plan", () => {
  it("is disabled without a goal", () => {
    setup({ goal: "" });
    const btn = screen.getByRole("button", { name: "AI 拆解" }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(btn.title).toContain("先設定大目標");
  });

  it("shows the proposal and writes NOTHING until the user adopts it", async () => {
    invokeAiComplete.mockResolvedValue(reply('["盤點 API","拆分登入模組","遷移資料庫"]'));
    setup();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "AI 拆解" })); });
    expect(invokeAiComplete).toHaveBeenCalledTimes(1);
    const [messages, , connId] = invokeAiComplete.mock.calls[0];
    expect(connId).toBe("milestone-plan-s1");
    expect(invokeAiComplete.mock.calls[0][1]).toBe(PLAIN_COMPLETION_SYSTEM_PROMPT);
    expect(messages[0].content).toContain(GOAL);
    const region = screen.getByRole("region", { name: "AI 提議的里程碑" });
    expect(within(region).getByText("拆分登入模組")).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.click(within(region).getByRole("button", { name: "採用" }));
    const next = onChange.mock.calls[0][0] as MilestoneState;
    expect(next.forGoal).toBe(GOAL);
    expect(next.items.map((i) => i.text)).toEqual(["盤點 API", "拆分登入模組", "遷移資料庫"]);
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("region", { name: "AI 提議的里程碑" })).toBeNull();
  });

  it("cancel discards the proposal without writing", async () => {
    invokeAiComplete.mockResolvedValue(reply('["A","B"]'));
    setup();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "AI 拆解" })); });
    fireEvent.click(within(screen.getByRole("region", { name: "AI 提議的里程碑" })).getByRole("button", { name: "取消" }));
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.queryByRole("region", { name: "AI 提議的里程碑" })).toBeNull();
  });

  it("planning again keeps the finished items and says so", async () => {
    invokeAiComplete.mockResolvedValue(reply('["拆分登入模組（新）","部署上線"]'));
    setup({ state: THREE });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "AI 拆解" })); });
    const region = screen.getByRole("region", { name: "AI 提議的里程碑" });
    expect(within(region).getByText("已完成的項目會保留。")).toBeTruthy();
    fireEvent.click(within(region).getByRole("button", { name: "採用" }));
    const next = onChange.mock.calls[0][0] as MilestoneState;
    expect(next.items.map((i) => i.text)).toEqual(["盤點 API", "拆分登入模組（新）", "部署上線"]);
    expect(next.items[0].id).toBe("a");
    expect(next.items[0].done).toBe(true);
  });

  it("shows a message and writes nothing when the AI fails or returns nothing usable", async () => {
    invokeAiComplete.mockRejectedValueOnce({ kind: "network", message: "x" });
    setup();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "AI 拆解" })); });
    expect(screen.getByRole("alert").textContent).toContain("操作失敗");
    invokeAiComplete.mockResolvedValueOnce(reply("抱歉我不知道"));
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "AI 拆解" })); });
    expect(screen.getByRole("alert").textContent).toContain("沒有回傳可用的里程碑");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("shows a busy label while waiting and blocks starting a second request", async () => {
    invokeAiComplete.mockImplementation(() => new Promise(() => {}));
    setup();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "AI 拆解" })); });
    expect((screen.getByRole("button", { name: "拆解中…" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole("button", { name: "檢查進度" }) as HTMLButtonElement).disabled).toBe(true);
    expect(invokeAiComplete).toHaveBeenCalledTimes(1);
  });

  it("two clicks in the same tick send only one request", async () => {
    invokeAiComplete.mockImplementation(() => new Promise(() => {}));
    setup();
    const btn = screen.getByRole("button", { name: "AI 拆解" });
    await act(async () => { btn.click(); btn.click(); });
    expect(invokeAiComplete).toHaveBeenCalledTimes(1);
  });

  it("drops a reply that arrives after the goal changed, and aborts the request", async () => {
    let resolveIt!: (v: unknown) => void;
    invokeAiComplete.mockImplementation(() => new Promise((r) => { resolveIt = r; }));
    const { rerender } = setup();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "AI 拆解" })); });
    rerender(<MilestoneList sessionId="s1" goal="另一個目標" state={undefined} onChange={onChange} getHistory={() => []} focusId={null} onFocus={onFocus} redact />);
    expect(abortAi).toHaveBeenCalledWith("milestone-plan-s1");
    await act(async () => { resolveIt(reply('["遲到的結果"]')); });
    expect(screen.queryByText("遲到的結果")).toBeNull();
    expect(screen.queryByRole("region", { name: "AI 提議的里程碑" })).toBeNull();
  });

  it("aborts an in-flight request when unmounted", async () => {
    invokeAiComplete.mockImplementation(() => new Promise(() => {}));
    const { unmount } = setup();
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "AI 拆解" })); });
    unmount();
    expect(abortAi).toHaveBeenCalledWith("milestone-plan-s1");
  });
});

describe("MilestoneList – focus", () => {
  it("marks the focused milestone and lets the user move the focus to another unfinished one", () => {
    setup({ state: THREE, focusId: "b" });
    expect(screen.getByText("焦點")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "設為焦點：拆分登入模組" })).toBeNull(); // 已經是焦點
    fireEvent.click(screen.getByRole("button", { name: "設為焦點：遷移資料庫" }));
    expect(onFocus).toHaveBeenCalledWith("c");
    expect(onChange).not.toHaveBeenCalled(); // 焦點不是資料，不寫進里程碑
  });

  it("offers no focus button on a finished milestone", () => {
    setup({ state: THREE, focusId: "b" });
    expect(screen.queryByRole("button", { name: "設為焦點：盤點 API" })).toBeNull();
  });

  it("shows no focus marker when nothing is focused (everything done)", () => {
    setup({ state: S([["a", "一", true]]), focusId: null });
    expect(screen.queryByText("焦點")).toBeNull();
  });
});

describe("MilestoneList – goal changed", () => {
  it("offers to plan again when the goal differs from the one the milestones were planned for", async () => {
    invokeAiComplete.mockResolvedValue(reply('["新方向一","新方向二"]'));
    setup({ state: THREE, goal: "完全不同的新目標" });
    expect(screen.getByText("目標已變更，要重新拆解嗎？")).toBeTruthy();
    expect(screen.getByText("盤點 API")).toBeTruthy(); // 不刪
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "重新拆解" })); });
    expect(screen.getByRole("region", { name: "AI 提議的里程碑" })).toBeTruthy();
  });

  it("shows no notice when the goal is unchanged", () => {
    setup({ state: THREE });
    expect(screen.queryByText("目標已變更，要重新拆解嗎？")).toBeNull();
  });
});

describe("MilestoneList – AI progress check", () => {
  it("is disabled when there is nothing left to check", () => {
    setup();
    expect((screen.getByRole("button", { name: "檢查進度" }) as HTMLButtonElement).disabled).toBe(true);
    const allDone = S([["a", "一", true], ["b", "二", true]]);
    setup({ state: allDone });
    expect((screen.getAllByRole("button", { name: "檢查進度" })[1] as HTMLButtonElement).disabled).toBe(true);
  });

  it("sends numbered milestones, the history and the current screen on a separate stream id", async () => {
    invokeAiComplete.mockResolvedValue(reply('{"done":[2],"note":"登入已完成"}'));
    setup({ state: THREE, getHistory: () => ["較早畫面一\n較早畫面二\n較早畫面三\n較早畫面四\n較早畫面五"] });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "檢查進度" })); });
    const [messages, , connId] = invokeAiComplete.mock.calls[0];
    expect(connId).toBe("milestone-check-s1");
    expect(invokeAiComplete.mock.calls[0][1]).toBe(PLAIN_COMPLETION_SYSTEM_PROMPT);
    const content = messages[0].content as string;
    expect(content).toContain("2. [未完成] 拆分登入模組");
    expect(content).toContain("較早畫面一");
    expect(content).toContain("CURRENT");
  });

  it("two clicks in the same tick send only one check request", async () => {
    invokeAiComplete.mockImplementation(() => new Promise(() => {}));
    setup({ state: THREE });
    const btn = screen.getByRole("button", { name: "檢查進度" });
    await act(async () => { btn.click(); btn.click(); });
    expect(invokeAiComplete).toHaveBeenCalledTimes(1);
  });

  it("only proposes: nothing is ticked until the user accepts each item", async () => {
    invokeAiComplete.mockResolvedValue(reply('{"done":[2,3],"note":"登入與資料庫都完成了"}'));
    setup({ state: THREE });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "檢查進度" })); });
    const region = screen.getByRole("region", { name: "AI 認為已完成：" });
    expect(within(region).getByText("登入與資料庫都完成了")).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();

    fireEvent.click(within(region).getByRole("button", { name: "採用：拆分登入模組" }));
    const next = onChange.mock.calls[0][0] as MilestoneState;
    expect(next.items.map((i) => i.done)).toEqual([true, true, false]);
    // 採用過的從提議裡拿掉；另一項還在
    expect(within(region).queryByRole("button", { name: "採用：拆分登入模組" })).toBeNull();
    expect(within(region).getByRole("button", { name: "採用：遷移資料庫" })).toBeTruthy();
  });

  it("skipping an item changes nothing, and the panel closes when nothing is left", async () => {
    invokeAiComplete.mockResolvedValue(reply('{"done":[3],"note":""}'));
    setup({ state: THREE });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "檢查進度" })); });
    fireEvent.click(screen.getByRole("button", { name: "略過：遷移資料庫" }));
    expect(onChange).not.toHaveBeenCalled();
    expect(screen.queryByRole("region", { name: "AI 認為已完成：" })).toBeNull();
  });

  it("says so when the AI found nothing to confirm", async () => {
    invokeAiComplete.mockResolvedValue(reply('{"done":[],"note":""}'));
    setup({ state: THREE });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "檢查進度" })); });
    expect(screen.getByText("AI 沒有發現可以確認完成的項目。")).toBeTruthy();
    expect(onChange).not.toHaveBeenCalled();
  });

  describe("secret redaction in the progress check", () => {
    const TOKEN = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    const sent = () => invokeAiComplete.mock.calls[0][0][0].content as string;

    it("masks secrets in the current and the earlier screens, and says how many", async () => {
      invokeAiComplete.mockResolvedValue(reply('{"done":[],"note":""}'));
      screenText = `目前畫面\npassword=hunter2hunter2\n結束`;
      setup({ state: THREE, getHistory: () => [`較早畫面\nAuthorization: Bearer ${TOKEN}`] });
      await act(async () => { fireEvent.click(screen.getByRole("button", { name: "檢查進度" })); });
      expect(sent()).not.toContain("hunter2hunter2");
      expect(sent()).not.toContain(TOKEN);
      expect(sent()).toContain("結束");
      expect(screen.getByText("已在送出前遮罩 2 處疑似敏感資訊")).toBeTruthy();
    });

    it("masks before truncating so no fragment survives the 8000-char cut", async () => {
      invokeAiComplete.mockResolvedValue(reply('{"done":[],"note":""}'));
      const suffix = "y ".repeat(3990).slice(0, 8000 - 20);
      screenText = `${"x ".repeat(2000)}${TOKEN}${suffix}`;
      setup({ state: THREE });
      await act(async () => { fireEvent.click(screen.getByRole("button", { name: "檢查進度" })); });
      expect(sent()).not.toContain(TOKEN.slice(-20));
      expect(sent()).not.toContain(TOKEN.slice(-12));
    });

    it("sends raw text and shows no count when redaction is off", async () => {
      invokeAiComplete.mockResolvedValue(reply('{"done":[],"note":""}'));
      screenText = `password=hunter2hunter2`;
      setup({ state: THREE, redact: false });
      await act(async () => { fireEvent.click(screen.getByRole("button", { name: "檢查進度" })); });
      expect(sent()).toContain("hunter2hunter2");
      expect(screen.queryByText(/已在送出前遮罩/)).toBeNull();
    });

    it("shows no count when nothing was masked", async () => {
      invokeAiComplete.mockResolvedValue(reply('{"done":[],"note":""}'));
      setup({ state: THREE });
      await act(async () => { fireEvent.click(screen.getByRole("button", { name: "檢查進度" })); });
      expect(screen.queryByText(/已在送出前遮罩/)).toBeNull();
    });
  });

  it("does not call the AI when the terminal has nothing readable", async () => {
    screenText = "  \n ";
    setup({ state: THREE });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "檢查進度" })); });
    expect(invokeAiComplete).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toContain("終端機沒有可讀的內容");
  });

  it("shows an error when the AI fails", async () => {
    invokeAiComplete.mockRejectedValueOnce({ kind: "network", message: "x" });
    setup({ state: THREE });
    await act(async () => { fireEvent.click(screen.getByRole("button", { name: "檢查進度" })); });
    expect(screen.getByRole("alert").textContent).toContain("操作失敗");
    await flush();
  });
});
