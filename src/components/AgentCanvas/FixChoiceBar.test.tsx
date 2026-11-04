import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { FixChoiceItem } from "../../ipc/tasks";
import { FixChoiceBar } from "./FixChoiceBar";

const item = (over: Partial<FixChoiceItem> = {}): FixChoiceItem => ({
  module_id: "m1",
  module_title: "A 登入",
  fix_id: "f1",
  round: 1,
  can_resume: true,
  remembered: null,
  ...over,
});

const renderBar = (items: FixChoiceItem[], onChoose = vi.fn().mockResolvedValue(undefined)) => {
  render(
    <LocaleProvider>
      <FixChoiceBar items={items} onChoose={onChoose} />
    </LocaleProvider>,
  );
  return onChoose;
};

describe("修正方式選擇列", () => {
  it("沒有等選擇的模組時什麼都不畫", () => {
    renderBar([]);
    expect(screen.queryByTestId("fixchoice-bar")).toBeNull();
  });

  it("每個等選擇的模組一列，寫明模組名與第幾輪", () => {
    renderBar([item(), item({ module_id: "m2", module_title: "B 搜尋", round: 2, fix_id: "f2" })]);
    expect(screen.getByText("《A 登入》被審核者退回（第 1 輪），怎麼修？")).toBeInTheDocument();
    expect(screen.getByText("《B 搜尋》被審核者退回（第 2 輪），怎麼修？")).toBeInTheDocument();
    expect(screen.getAllByTestId("fixchoice-fresh")).toHaveLength(2);
  });

  it("按「新開修正卡」帶著模組與預設勾選的記住", async () => {
    const onChoose = renderBar([item()]);
    fireEvent.click(screen.getByTestId("fixchoice-fresh"));
    await waitFor(() => expect(onChoose).toHaveBeenCalledWith("m1", "fresh", true));
  });

  it("按「接回原對話」；取消勾選就只這次有效", async () => {
    const onChoose = renderBar([item()]);
    expect(screen.getByTestId("fixchoice-remember")).toBeChecked();
    fireEvent.click(screen.getByTestId("fixchoice-remember"));
    fireEvent.click(screen.getByTestId("fixchoice-resume"));
    await waitFor(() => expect(onChoose).toHaveBeenCalledWith("m1", "resume", false));
  });

  it("勾選狀態各模組獨立", async () => {
    const onChoose = renderBar([item(), item({ module_id: "m2", module_title: "B", fix_id: "f2" })]);
    fireEvent.click(screen.getAllByTestId("fixchoice-remember")[0]);
    fireEvent.click(screen.getAllByTestId("fixchoice-fresh")[1]);
    await waitFor(() => expect(onChoose).toHaveBeenCalledWith("m2", "fresh", true));
  });

  it("接不回時「接回原對話」停用並用 title 說明原因，點了也不會送出", () => {
    const onChoose = renderBar([item({ can_resume: false })]);
    const btn = screen.getByTestId("fixchoice-resume");
    expect(btn).toBeDisabled();
    expect(btn).toHaveAttribute("title", "找不到原本的對話檔（可能被清掉了），沒辦法接回。");
    fireEvent.click(btn);
    expect(onChoose).not.toHaveBeenCalled();
    expect(screen.getByTestId("fixchoice-fresh")).toBeEnabled();
  });

  it("記過選擇的模組顯示上次的選擇，並誠實說明之後怎麼處理", () => {
    renderBar([item({ remembered: "resume", round: 2 })]);
    expect(screen.getByText("上次的選擇：接回原對話")).toBeInTheDocument();
    expect(screen.getByText(/第 2 輪起還是會再問一次/)).toBeInTheDocument();
  });

  it("送出失敗時顯示原因、按鈕恢復可按", async () => {
    const onChoose = vi.fn().mockRejectedValue("找不到原本的對話檔");
    renderBar([item()], onChoose);
    fireEvent.click(screen.getByTestId("fixchoice-resume"));
    expect(await screen.findByText("找不到原本的對話檔")).toBeInTheDocument();
    expect(screen.getByTestId("fixchoice-resume")).toBeEnabled();
  });
});
