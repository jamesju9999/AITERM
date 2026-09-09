import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { TaskLabelGroup } from "./TaskLabelGroup";

describe("TaskLabelGroup", () => {
  it("預設展開，看得到 children", () => {
    render(
      <TaskLabelGroup label="緊急" count={2}>
        <div data-testid="child">卡片</div>
      </TaskLabelGroup>,
    );
    expect(screen.getByTestId("child")).toBeInTheDocument();
    expect(screen.getByText("緊急")).toBeInTheDocument();
    expect(screen.getByText("(2)")).toBeInTheDocument();
  });

  it("點標頭切換摺疊，children 消失；再點一次恢復", async () => {
    render(
      <TaskLabelGroup label="緊急" count={1}>
        <div data-testid="child">卡片</div>
      </TaskLabelGroup>,
    );
    const header = screen.getByRole("button");
    await userEvent.click(header);
    expect(screen.queryByTestId("child")).not.toBeInTheDocument();
    await userEvent.click(header);
    expect(screen.getByTestId("child")).toBeInTheDocument();
  });
});
