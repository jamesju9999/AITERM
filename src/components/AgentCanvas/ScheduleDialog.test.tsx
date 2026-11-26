import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { AgentRow } from "../../ipc/agents";
import type { ScheduleView } from "../../ipc/schedules";
import { invokeAiComplete } from "../../ipc/ai";
import { ScheduleDialog } from "./ScheduleDialog";

vi.mock("../../ipc/ai", () => ({ invokeAiComplete: vi.fn() }));
vi.mock("../../ipc/provider", () => ({
  listProviders: vi.fn().mockResolvedValue([{ id: "p1", name: "P1", is_default: true, models: [] }]),
}));

const agent = (id: string, kind: AgentRow["kind"], name: string) => ({ id, kind, name }) as AgentRow;
const agents = [agent("c1", "coder", "工程師A"), agent("r1", "reviewer", "審核者"), agent("v1", "verifier", "驗收者")];

const existing: ScheduleView = {
  id: "s1", name: "舊的", agent_id: "v1", instruction: "舊指令", schedule_kind: "daily", schedule_json: '{"times":["09:30"]}',
  enabled: true, failures_count_as_failed: false, canvas_x: null, canvas_y: null, last_fired_at: 0, last_module_id: null,
  fail_streak: 0, skip_streak: 0, disabled_reason: null, runs: [],
};

function mount(props: Partial<React.ComponentProps<typeof ScheduleDialog>> = {}) {
  const onSave = vi.fn().mockResolvedValue(undefined);
  const onClose = vi.fn();
  render(
    <LocaleProvider>
      <ScheduleDialog agents={agents} initial={null} onSave={onSave} onClose={onClose} {...props} />
    </LocaleProvider>,
  );
  return { onSave, onClose };
}

describe("ScheduleDialog server time hint", () => {
  it("shows the server-time-zone hint only for shared boards", () => {
    mount({ serverTime: true });
    expect(screen.getByTestId("sched-server-time-hint")).toHaveTextContent("server 主機的時區");
  });
  it("does not show it for local projects", () => {
    mount();
    expect(screen.queryByTestId("sched-server-time-hint")).toBeNull();
  });
});

async function fillBasics() {
  await userEvent.type(screen.getByLabelText("名稱"), "每日測試");
  await userEvent.type(screen.getByLabelText("固定指令"), "跑完整測試");
}

describe("ScheduleDialog", () => {
  it("角色下拉只列工程師與驗收者", () => {
    mount();
    const options = screen.getAllByRole("option").map((o) => o.textContent);
    expect(options).toContain("工程師A");
    expect(options).toContain("驗收者");
    expect(options).not.toContain("審核者");
  });

  it("名稱與指令都空白時不能儲存", async () => {
    const { onSave } = mount();
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(onSave).not.toHaveBeenCalled();
  });

  it("每天模式：儲存時送出時間點", async () => {
    const { onSave, onClose } = mount();
    await fillBasics();
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(onSave).toHaveBeenCalledWith(
      expect.objectContaining({ name: "每日測試", agent_id: "c1", instruction: "跑完整測試", schedule_kind: "daily", spec: { times: ["08:00"] } }),
    );
    expect(onClose).toHaveBeenCalled();
  });

  it("切到每週才出現星期勾選，切到間隔才出現分鐘欄位", async () => {
    mount();
    expect(screen.queryByText("星期")).toBeNull();
    await userEvent.selectOptions(screen.getByLabelText("週期"), "weekly");
    expect(screen.getByText("星期")).toBeTruthy();
    await userEvent.selectOptions(screen.getByLabelText("週期"), "interval");
    expect(screen.getByLabelText("分鐘")).toBeTruthy();
    expect(screen.queryByText("星期")).toBeNull();
  });

  it("每週模式：預設平日，送出勾選的星期", async () => {
    const { onSave } = mount();
    await fillBasics();
    await userEvent.selectOptions(screen.getByLabelText("週期"), "weekly");
    await userEvent.click(screen.getByLabelText("週一")); // 取消週一
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ schedule_kind: "weekly", spec: { times: ["08:00"], days: [2, 3, 4, 5] } }));
  });

  it("間隔小於 5 分鐘不能儲存，5 分鐘可以", async () => {
    const { onSave } = mount();
    await fillBasics();
    await userEvent.selectOptions(screen.getByLabelText("週期"), "interval");
    const mins = screen.getByLabelText("分鐘");
    await userEvent.clear(mins);
    await userEvent.type(mins, "4");
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(onSave).not.toHaveBeenCalled();
    await userEvent.clear(mins);
    await userEvent.type(mins, "5");
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ schedule_kind: "interval", spec: { every_minutes: 5, window: null } }));
  });

  it("儲存失敗時顯示錯誤、不關閉", async () => {
    const onSave = vi.fn().mockRejectedValue("目標角色已不存在");
    const { onClose } = mount({ onSave });
    await fillBasics();
    await userEvent.click(screen.getByRole("button", { name: "儲存" }));
    expect(await screen.findByText("目標角色已不存在")).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("編輯既有定時器：帶入原值，可立即執行，可啟停", async () => {
    const onRunNow = vi.fn().mockResolvedValue("card1");
    const onToggleEnabled = vi.fn().mockResolvedValue(undefined);
    mount({ initial: existing, onRunNow, onToggleEnabled });
    expect((screen.getByLabelText("名稱") as HTMLInputElement).value).toBe("舊的");
    await userEvent.click(screen.getByRole("button", { name: "立即執行一次" }));
    expect(onRunNow).toHaveBeenCalled();
    expect(await screen.findByText("已建立一輪工作")).toBeTruthy();
    await userEvent.click(screen.getByLabelText("啟用"));
    expect(onToggleEnabled).toHaveBeenCalledWith(false);
  });

  it("立即執行被擋下（回空字串）時說明原因", async () => {
    mount({ initial: existing, onRunNow: vi.fn().mockResolvedValue("") });
    await userEvent.click(screen.getByRole("button", { name: "立即執行一次" }));
    expect(await screen.findByText("上一輪還沒結案，這次沒有建立")).toBeTruthy();
  });

  it("新增時沒有立即執行與刪除按鈕", () => {
    mount({ onRunNow: vi.fn(), onDelete: vi.fn() });
    expect(screen.queryByRole("button", { name: "立即執行一次" })).toBeNull();
    expect(screen.queryByRole("button", { name: "刪除" })).toBeNull();
  });

  describe("固定指令的 AI 潤飾", () => {
    beforeEach(() => vi.mocked(invokeAiComplete).mockReset());

    it("指令空白時不能潤飾", () => {
      mount();
      expect(screen.getByTestId("sched-refine")).toBeDisabled();
    });

    it("潤飾後換成 AI 的內容、保留一鍵還原，且只改指令欄", async () => {
      vi.mocked(invokeAiComplete).mockResolvedValue({ content: "```\n每次執行完整測試並回報失敗項目\n```" } as never);
      mount();
      await fillBasics();
      await userEvent.click(screen.getByTestId("sched-refine"));
      await waitFor(() => expect(screen.getByLabelText("固定指令")).toHaveValue("每次執行完整測試並回報失敗項目"));
      expect(vi.mocked(invokeAiComplete).mock.calls[0][0][0].content).toContain("跑完整測試");
      expect(screen.getByLabelText("名稱")).toHaveValue("每日測試");
      await userEvent.click(screen.getByTestId("sched-refine-undo"));
      expect(screen.getByLabelText("固定指令")).toHaveValue("跑完整測試");
      expect(screen.queryByTestId("sched-refine-undo")).toBeNull();
    });

    it("AI 回空內容時顯示錯誤並保留原文", async () => {
      vi.mocked(invokeAiComplete).mockResolvedValue({ content: "  " } as never);
      mount();
      await fillBasics();
      await userEvent.click(screen.getByTestId("sched-refine"));
      expect(await screen.findByText(/AI 沒有回覆/)).toBeInTheDocument();
      expect(screen.getByLabelText("固定指令")).toHaveValue("跑完整測試");
    });
  });

  it("欄位沿用 AITerm 共用的輸入樣式（跟其他對話框一致）", () => {
    mount();
    expect(screen.getByLabelText("名稱")).toHaveClass("task-field-input");
    expect(screen.getByLabelText("固定指令")).toHaveClass("task-field-input");
    expect(screen.getByLabelText("叫起的角色")).toHaveClass("task-field-input");
  });
});
