import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

vi.mock("react-router-dom", async () => {
  const actual = await vi.importActual<typeof import("react-router-dom")>("react-router-dom");
  return { ...actual, useNavigate: () => vi.fn() };
});

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TabBar } from "./index";

function renderBar(props: Partial<React.ComponentProps<typeof TabBar>> = {}) {
  return render(
    <LocaleProvider>
      <TabBar
        tabs={[]}
        activeId={""}
        onSelect={() => {}}
        onClose={() => {}}
        onAdd={() => {}}
        onRename={() => {}}
        onReorder={() => {}}
        isSidebarOpen
        onToggle={() => {}}
        width={220}
        {...props}
      />
    </LocaleProvider>,
  );
}

describe("TabBar task board button", () => {
  it("renders when onBoard is provided and calls it on click", async () => {
    const onBoard = vi.fn();
    renderBar({ onBoard, boardActive: false });
    const btn = screen.getByRole("button", { name: /工作看板|Task Board/ });
    await userEvent.click(btn);
    expect(onBoard).toHaveBeenCalled();
  });

  it("marks itself active when boardActive is true", () => {
    renderBar({ onBoard: () => {}, boardActive: true });
    expect(screen.getByRole("button", { name: /工作看板|Task Board/ })).toHaveClass("active");
  });
});
