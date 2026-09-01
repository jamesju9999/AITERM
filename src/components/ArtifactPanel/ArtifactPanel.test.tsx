import { useEffect } from "react";
import { describe, expect, it } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import {
  ArtifactPanelProvider,
  useArtifactPanel,
  type Artifact,
} from "../../contexts/ArtifactPanelContext";
import { ArtifactPanel } from "./ArtifactPanel";

function ShowOnMount({ artifact }: { artifact: Artifact }) {
  const { showArtifact } = useArtifactPanel();
  useEffect(() => {
    showArtifact(artifact);
  }, [artifact, showArtifact]);
  return null;
}

describe("ArtifactPanel", () => {
  it("renders nothing when there is no active artifact", () => {
    const { container } = render(
      <ArtifactPanelProvider>
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    expect(container.querySelector(".aiterm-artifact-panel")).toBeNull();
  });

  it("renders an html artifact inside an iframe with the artifact's title", () => {
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "1", kind: "html", title: "Brief", content: "<p>hi</p>" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByText("Brief")).toBeInTheDocument();
    expect(document.querySelector("iframe")).not.toBeNull();
  });

  it("shows an error message when chart content is not valid JSON", () => {
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "2", kind: "chart", title: "Bad", content: "not json" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByText("圖表資料格式錯誤，無法解析。")).toBeInTheDocument();
  });

  it("clicking close clears the active artifact", () => {
    render(
      <ArtifactPanelProvider>
        <ShowOnMount artifact={{ id: "1", kind: "html", title: "Brief", content: "<p>hi</p>" }} />
        <ArtifactPanel />
      </ArtifactPanelProvider>,
    );
    fireEvent.click(screen.getByTitle("關閉文件面板"));
    expect(screen.queryByText("Brief")).not.toBeInTheDocument();
  });
});
