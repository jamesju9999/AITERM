import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TopMoreMenu } from "./TopMoreMenu";

function mount(onAddSchedule?: () => void) {
  render(
    <LocaleProvider>
      <TopMoreMenu cap={null} onHelp={() => {}} onAddSchedule={onAddSchedule} />
    </LocaleProvider>,
  );
}

describe("TopMoreMenu", () => {
  it("有 onAddSchedule 時選單出現「新增定時器」，點下去呼叫並收起選單", async () => {
    const onAdd = vi.fn();
    mount(onAdd);
    await userEvent.click(screen.getByRole("button", { name: "更多" }));
    await userEvent.click(screen.getByRole("menuitem", { name: /新增定時器/ }));
    expect(onAdd).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("menuitem", { name: /新增定時器/ })).toBeNull();
  });

  it("沒有 onAddSchedule 時不顯示", async () => {
    mount();
    await userEvent.click(screen.getByRole("button", { name: "更多" }));
    expect(screen.queryByRole("menuitem", { name: /新增定時器/ })).toBeNull();
  });
});
