import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { ArtifactPending } from "./ArtifactPending";

describe("ArtifactPending", () => {
  it("says a document is being produced", () => {
    render(<ArtifactPending kind="html" />);
    expect(screen.getByText(/文件產生中/)).toBeInTheDocument();
  });

  it("says a chart is being produced", () => {
    render(<ArtifactPending kind="chart" />);
    expect(screen.getByText(/圖表產生中/)).toBeInTheDocument();
  });

  // 這是它存在的理由：產生一份 HTML 報告可能是好幾千個 token，期間畫面若只有
  // 一段「我已完成整理」的文字然後長時間沒動靜，使用者會以為當掉了。aria-busy
  // 讓輔助技術也知道還在進行中。
  it("marks itself busy while the artifact is still streaming", () => {
    const { container } = render(<ArtifactPending kind="html" />);
    expect(container.querySelector('[aria-busy="true"]')).not.toBeNull();
  });
});
