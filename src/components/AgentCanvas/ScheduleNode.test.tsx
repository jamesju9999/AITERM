import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { ScheduleView } from "../../ipc/schedules";
import { ScheduleNode } from "./ScheduleNode";

const sched = (over: Partial<ScheduleView> = {}): ScheduleView => ({
  id: "s1", name: "每日測試", agent_id: "c1", instruction: "x", schedule_kind: "daily", schedule_json: '{"times":["08:00"]}',
  enabled: true, failures_count_as_failed: false, canvas_x: null, canvas_y: null, last_fired_at: 0, last_module_id: null,
  fail_streak: 0, skip_streak: 0, disabled_reason: null, runs: [], ...over,
});

function mount(s: ScheduleView) {
  const onEdit = vi.fn();
  render(
    <LocaleProvider>
      <ScheduleNode schedule={s} x={10} y={20} onEdit={onEdit} />
    </LocaleProvider>,
  );
  return onEdit;
}

describe("ScheduleNode", () => {
  it("顯示名稱、週期摘要與啟用狀態", () => {
    mount(sched());
    expect(screen.getByText("⏰ 每日測試")).toBeTruthy();
    expect(screen.getByText("每天 08:00")).toBeTruthy();
    expect(screen.getByText("啟用中")).toBeTruthy();
  });
  it("連續失敗停用顯示原因", () => {
    mount(sched({ enabled: false, disabled_reason: "連續失敗" }));
    expect(screen.getByText("已因連續失敗停用")).toBeTruthy();
  });
  it("角色消失顯示原因", () => {
    mount(sched({ enabled: false, disabled_reason: "角色已不存在" }));
    expect(screen.getByText("角色已不存在，已停用")).toBeTruthy();
  });
  it("角色因卡片失敗被暫停：說明原因與該怎麼辦", () => {
    mount(sched({ enabled: false, disabled_reason: "角色已暫停" }));
    expect(screen.getByText("負責的角色被暫停，已停用（處理好卡片後重新啟用）")).toBeTruthy();
  });
  it("上一輪卡住顯示提示", () => {
    mount(sched({ skip_streak: 3 }));
    expect(screen.getByText("上一輪卡住了")).toBeTruthy();
  });
  it("最近一輪結果顯示在節點上", () => {
    mount(sched({ runs: [{ schedule_id: "s1", fired_at: 1, module_id: null, result: "failed" }] }));
    expect(screen.getByText("失敗")).toBeTruthy();
  });
  it("點節點開啟編輯", async () => {
    const onEdit = mount(sched());
    await userEvent.click(screen.getByTestId("schedule-node"));
    expect(onEdit).toHaveBeenCalled();
  });

  it("按下滑鼠時通知拖曳（讓畫布可以拖動節點）", () => {
    const onDragStart = vi.fn();
    render(
      <LocaleProvider>
        <ScheduleNode schedule={sched()} x={10} y={20} onEdit={() => {}} onDragStart={onDragStart} />
      </LocaleProvider>,
    );
    fireEvent.mouseDown(screen.getByTestId("schedule-node"));
    expect(onDragStart).toHaveBeenCalledTimes(1);
  });
});
