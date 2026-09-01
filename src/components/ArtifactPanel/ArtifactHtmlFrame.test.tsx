import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { ArtifactHtmlFrame } from "./ArtifactHtmlFrame";

describe("ArtifactHtmlFrame", () => {
  it("renders an iframe with the given HTML as srcDoc and the given title", () => {
    const { container } = render(<ArtifactHtmlFrame html="<p>hello</p>" title="Brief" />);
    const frame = container.querySelector("iframe");
    expect(frame).not.toBeNull();
    expect(frame).toHaveAttribute("srcdoc", "<p>hello</p>");
    expect(frame).toHaveAttribute("title", "Brief");
  });

  // 這是整份設計最不能退讓的安全底線：allow-scripts 沒有搭配
  // allow-same-origin，沙盒才會把 iframe 隔成不透明來源，內容碰不到主視窗
  // 的 DOM/localStorage/Tauri IPC。獨立寫一個測試，防止未來被「順手」加回
  // allow-same-origin。
  it("sandbox allows scripts but never allow-same-origin", () => {
    const { container } = render(<ArtifactHtmlFrame html="<script>1</script>" title="x" />);
    const frame = container.querySelector("iframe")!;
    const tokens = (frame.getAttribute("sandbox") ?? "").split(/\s+/).filter(Boolean);
    expect(tokens).toContain("allow-scripts");
    expect(tokens).not.toContain("allow-same-origin");
  });
});
