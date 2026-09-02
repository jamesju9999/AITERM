import { useEffect } from "react";
import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import {
  ArtifactPanelProvider,
  useArtifactPanel,
  type Artifact,
} from "../../contexts/ArtifactPanelContext";
import { ArtifactSplit } from "./ArtifactSplit";

function ShowOnMount({ artifact }: { artifact: Artifact }) {
  const { showArtifact } = useArtifactPanel();
  useEffect(() => { showArtifact(artifact); }, [artifact, showArtifact]);
  return null;
}

const htmlArtifact: Artifact = {
  id: "1", kind: "html", title: "Brief", content: "<p>hi</p>",
};

describe("ArtifactSplit", () => {
  it("renders only its children when there is no artifact", () => {
    const { container } = render(
      <ArtifactPanelProvider>
        <ArtifactSplit><div>CHAT</div></ArtifactSplit>
      </ArtifactPanelProvider>,
    );
    expect(screen.getByText("CHAT")).toBeInTheDocument();
    expect(container.querySelector(".aiterm-artifact-panel")).toBeNull();
    expect(container.querySelector(".aiterm-artifact-resizer")).toBeNull();
    expect(container.querySelector(".aiterm-artifact-split--active")).toBeNull();
  });

  it("renders the panel and the resizer alongside its children when an artifact is active", () => {
    const { container } = render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={htmlArtifact} />
        <ArtifactSplit><div>CHAT</div></ArtifactSplit>
      </ArtifactPanelProvider>,
    );
    expect(screen.getByText("CHAT")).toBeInTheDocument();
    expect(container.querySelector(".aiterm-artifact-panel")).not.toBeNull();
    expect(container.querySelector(".aiterm-artifact-resizer")).not.toBeNull();
    expect(container.querySelector(".aiterm-artifact-split--active")).not.toBeNull();
  });

  // 這是這個元件存在的理由：右欄是 iframe，用 window mousemove 監聽的話游標一進
  // iframe 就收不到事件、拖曳會頓。pointer capture 才不會斷（見
  // docs/superpowers/specs/2026-09-01-artifact-panel-design.md 與 commit 544d935）。
  it("drags with pointer capture so the iframe cannot swallow the events", () => {
    const { container } = render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={htmlArtifact} />
        <ArtifactSplit><div>CHAT</div></ArtifactSplit>
      </ArtifactPanelProvider>,
    );
    const grip = container.querySelector(".aiterm-artifact-resizer") as HTMLElement;
    let captured = false;
    grip.setPointerCapture = () => { captured = true; };
    grip.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerId: 1 }));
    expect(captured).toBe(true);
  });
});
