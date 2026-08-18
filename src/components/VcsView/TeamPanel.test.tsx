import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { TeamPanel } from "./TeamPanel";
import { LocaleProvider } from "../../contexts/LocaleContext";
import type { ActiveFeature } from "../../ipc/vcs";

const FEATURE: ActiveFeature = {
  number: 7,
  title: "登入頁優化",
  author: "alice",
  draft: true,
  url: "https://github.com/acme/widget/pull/7",
  updated_at: "2026-08-17T00:00:00Z",
  head_ref: "feature/login-optimize",
  files: ["src/Login.tsx", "src/api/auth.ts"],
};

function renderPanel(props: Partial<React.ComponentProps<typeof TeamPanel>> = {}) {
  return render(
    <LocaleProvider>
      <TeamPanel
        features={[FEATURE]}
        loading={false}
        onRefresh={vi.fn()}
        onStartFeature={vi.fn()}
        onFinishFeature={vi.fn()}
        {...props}
      />
    </LocaleProvider>
  );
}

describe("TeamPanel", () => {
  it("shows each active feature's title and author", () => {
    renderPanel();
    expect(screen.getByText("登入頁優化")).toBeInTheDocument();
    expect(screen.getByText("alice")).toBeInTheDocument();
  });

  it("expands to show changed files on click", () => {
    renderPanel();
    fireEvent.click(screen.getByText("登入頁優化"));
    expect(screen.getByText("src/Login.tsx")).toBeInTheDocument();
    expect(screen.getByText("src/api/auth.ts")).toBeInTheDocument();
  });

  it("calls onRefresh when the refresh button is clicked", () => {
    const onRefresh = vi.fn();
    renderPanel({ onRefresh });
    fireEvent.click(screen.getByRole("button", { name: /重新整理/ }));
    expect(onRefresh).toHaveBeenCalled();
  });

  it("calls onStartFeature when '開始新功能' is clicked", () => {
    const onStartFeature = vi.fn();
    renderPanel({ onStartFeature });
    fireEvent.click(screen.getByRole("button", { name: "開始新功能" }));
    expect(onStartFeature).toHaveBeenCalled();
  });

  it("shows an empty state when there are no active features", () => {
    renderPanel({ features: [] });
    expect(screen.getByText(/目前沒有進行中的功能/)).toBeInTheDocument();
  });

  it("calls onFinishFeature when finish button is clicked on a draft feature", () => {
    const onFinishFeature = vi.fn();
    renderPanel({ onFinishFeature });
    fireEvent.click(screen.getByText("登入頁優化")); // expand
    fireEvent.click(screen.getByRole("button", { name: "完成，送審" }));
    expect(onFinishFeature).toHaveBeenCalledWith(FEATURE);
  });

  it("does not show the finish button for a feature that is already in review (not draft)", () => {
    const inReview = { ...FEATURE, draft: false };
    renderPanel({ features: [inReview] });
    fireEvent.click(screen.getByText("登入頁優化")); // expand
    expect(screen.queryByRole("button", { name: "完成，送審" })).not.toBeInTheDocument();
  });

  it("collapses the file list when the expanded row is clicked again", () => {
    renderPanel();
    fireEvent.click(screen.getByText("登入頁優化")); // expand
    expect(screen.getByText("src/Login.tsx")).toBeInTheDocument();
    fireEvent.click(screen.getByText("登入頁優化")); // collapse
    expect(screen.queryByText("src/Login.tsx")).not.toBeInTheDocument();
  });
});
