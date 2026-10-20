import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { FormationsState } from "../../ipc/formations";
import { FormationMenu } from "./FormationMenu";

const base: FormationsState = {
  formations: [
    { id: "f1", name: "盤點", agent_count: 5 },
    { id: "f2", name: "轉換", agent_count: 3 },
  ],
  active: "f1",
  pending: null,
  busy: [],
  log: [],
};

const view = (state: FormationsState = base) => {
  const h = {
    onSwitch: vi.fn(),
    onCreate: vi.fn(),
    onSaveAs: vi.fn(),
    onRename: vi.fn(),
    onDelete: vi.fn(),
    onCancelPending: vi.fn(),
    onTemplates: vi.fn(),
    onClearTeam: vi.fn(),
  };
  render(
    <LocaleProvider>
      <FormationMenu state={state} {...h} />
    </LocaleProvider>,
  );
  return h;
};

describe("FormationMenu", () => {
  it("按鈕顯示目前的編制；清單裡目前那組打勾、顯示人數", () => {
    view();
    expect(screen.getByTestId("ac-formation").textContent).toContain("編制：盤點");
    fireEvent.click(screen.getByTestId("ac-formation"));
    const items = screen.getAllByRole("menuitem");
    expect(items[0].textContent).toContain("✓");
    expect(items[0].textContent).toContain("5 位");
    expect(items[1].textContent).not.toContain("✓");
  });

  it("點另一組就切換；點目前那組不切換", () => {
    const h = view();
    fireEvent.click(screen.getByTestId("ac-formation"));
    fireEvent.click(screen.getByRole("menuitem", { name: /轉換/ }));
    expect(h.onSwitch).toHaveBeenCalledWith("f2");
    fireEvent.click(screen.getByTestId("ac-formation"));
    fireEvent.click(screen.getByRole("menuitem", { name: /盤點/ }));
    expect(h.onSwitch).toHaveBeenCalledTimes(1);
  });

  it("新增空白編制：輸入名稱按 Enter；空白名稱不送", () => {
    const h = view();
    fireEvent.click(screen.getByTestId("ac-formation"));
    fireEvent.click(screen.getByRole("menuitem", { name: /新增空白編制/ }));
    const input = screen.getByLabelText(/編制名稱/);
    fireEvent.keyDown(input, { key: "Enter" });
    expect(h.onCreate).not.toHaveBeenCalled();
    fireEvent.change(input, { target: { value: " 實測 " } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(h.onCreate).toHaveBeenCalledWith("實測");
  });

  it("另存與改名", () => {
    const h = view();
    fireEvent.click(screen.getByTestId("ac-formation"));
    fireEvent.click(screen.getByRole("menuitem", { name: /另存成新編制/ }));
    fireEvent.change(screen.getByLabelText(/編制名稱/), { target: { value: "盤點二" } });
    fireEvent.keyDown(screen.getByLabelText(/編制名稱/), { key: "Enter" });
    expect(h.onSaveAs).toHaveBeenCalledWith("盤點二");

    fireEvent.click(screen.getByTestId("ac-formation"));
    fireEvent.click(screen.getByRole("menuitem", { name: /重新命名/ }));
    const input = screen.getByLabelText(/編制名稱/) as HTMLInputElement;
    expect(input.value).toBe("盤點");
    fireEvent.change(input, { target: { value: "第一階段" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(h.onRename).toHaveBeenCalledWith("f1", "第一階段");
  });

  it("目前那組不能刪；別組可以", () => {
    const h = view();
    fireEvent.click(screen.getByTestId("ac-formation"));
    expect(screen.queryByRole("button", { name: "刪除編制「盤點」" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "刪除編制「轉換」" }));
    expect(h.onDelete).toHaveBeenCalledWith("f2", "轉換");
  });

  it("等待切換時說明在等誰，可以取消", () => {
    const h = view({ ...base, pending: "f2", busy: ["工程師1", "審核者"] });
    expect(screen.getByTestId("ac-formation-pending").textContent).toContain("等 工程師1、審核者 做完目前的工作後切換到「轉換」");
    fireEvent.click(screen.getByRole("button", { name: "取消切換" }));
    expect(h.onCancelPending).toHaveBeenCalled();
  });

  it("團隊範本與清空團隊在選單裡；清空用紅字放最後", () => {
    const h = view();
    fireEvent.click(screen.getByTestId("ac-formation"));
    const items = screen.getAllByRole("menuitem");
    expect(items[items.length - 1]).toBe(screen.getByTestId("ac-clear-team"));
    expect(screen.getByTestId("ac-clear-team").className).toContain("ac-form-danger");
    fireEvent.click(screen.getByTestId("ac-templates"));
    expect(h.onTemplates).toHaveBeenCalled();
    expect(screen.queryByRole("menu")).toBeNull();
    fireEvent.click(screen.getByTestId("ac-formation"));
    fireEvent.click(screen.getByTestId("ac-clear-team"));
    expect(h.onClearTeam).toHaveBeenCalled();
  });

  it("沒有任何編制時顯示未命名", () => {
    view({ ...base, formations: [], active: null });
    expect(screen.getByTestId("ac-formation").textContent).toContain("編制：未命名");
  });
});
