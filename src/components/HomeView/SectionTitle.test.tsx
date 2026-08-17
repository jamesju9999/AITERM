import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { SectionTitle } from "./SectionTitle";

describe("SectionTitle", () => {
  it("有 count 時顯示", () => {
    render(
      <SectionTitle icon={<span data-testid="icon" />} count="3 個分頁">
        標題
      </SectionTitle>,
    );
    expect(screen.getByText("3 個分頁")).toBeInTheDocument();
  });

  it("沒有 count 時不顯示", () => {
    const { container } = render(<SectionTitle icon={<span data-testid="icon" />}>標題</SectionTitle>);
    expect(container.querySelector(".home-section-count")).toBeNull();
  });

  it("顯示傳入的標題文字與圖示", () => {
    render(
      <SectionTitle icon={<span data-testid="icon" />}>標題</SectionTitle>,
    );
    expect(screen.getByText("標題")).toBeInTheDocument();
    expect(screen.getByTestId("icon")).toBeInTheDocument();
  });
});
