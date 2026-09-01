import { describe, expect, it } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ArtifactPanelProvider, useArtifactPanel } from "../../contexts/ArtifactPanelContext";
import { ArtifactBlockCard } from "./ArtifactBlockCard";

function ActiveTitle() {
  const { activeArtifact } = useArtifactPanel();
  return <div data-testid="active-title">{activeArtifact?.title ?? "none"}</div>;
}

describe("ArtifactBlockCard", () => {
  it("registers itself into the artifact panel on mount", () => {
    render(
      <ArtifactPanelProvider>
        <ArtifactBlockCard kind="html" content="<title>Brief</title><p>hi</p>" />
        <ActiveTitle />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByTestId("active-title")).toHaveTextContent("Brief");
  });

  it("falls back to a generic title when html has no <title>", () => {
    render(
      <ArtifactPanelProvider>
        <ArtifactBlockCard kind="html" content="<p>hi</p>" />
        <ActiveTitle />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByTestId("active-title")).toHaveTextContent("文件");
  });

  it("reads the title from a chart spec's JSON", () => {
    render(
      <ArtifactPanelProvider>
        <ArtifactBlockCard kind="chart" content='{"title":"Sales by Month"}' />
        <ActiveTitle />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByTestId("active-title")).toHaveTextContent("Sales by Month");
  });

  it("falls back to a generic title when chart content is not valid JSON", () => {
    render(
      <ArtifactPanelProvider>
        <ArtifactBlockCard kind="chart" content="not json" />
        <ActiveTitle />
      </ArtifactPanelProvider>,
    );
    expect(screen.getByTestId("active-title")).toHaveTextContent("圖表");
  });

  it("clicking the card re-shows the artifact", () => {
    render(
      <ArtifactPanelProvider>
        <ArtifactBlockCard kind="html" content="<title>Brief</title>" />
        <ActiveTitle />
      </ArtifactPanelProvider>,
    );
    fireEvent.click(screen.getByRole("button"));
    expect(screen.getByTestId("active-title")).toHaveTextContent("Brief");
  });
});
