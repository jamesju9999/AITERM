import { describe, expect, it } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ArtifactPanelProvider, useArtifactPanel } from "./ArtifactPanelContext";

function Probe() {
  const { activeArtifact, showArtifact, clearArtifact } = useArtifactPanel();
  return (
    <div>
      <div data-testid="active">
        {activeArtifact ? `${activeArtifact.kind}:${activeArtifact.title}` : "none"}
      </div>
      <button onClick={() => showArtifact({ id: "1", kind: "html", title: "Brief", content: "<p>hi</p>" })}>
        show-a
      </button>
      <button onClick={() => showArtifact({ id: "2", kind: "chart", title: "Sales", content: "{}" })}>
        show-b
      </button>
      <button onClick={clearArtifact}>clear</button>
    </div>
  );
}

describe("ArtifactPanelContext", () => {
  it("starts with no active artifact", () => {
    render(<ArtifactPanelProvider><Probe /></ArtifactPanelProvider>);
    expect(screen.getByTestId("active")).toHaveTextContent("none");
  });

  it("showArtifact sets the active artifact", () => {
    render(<ArtifactPanelProvider><Probe /></ArtifactPanelProvider>);
    fireEvent.click(screen.getByText("show-a"));
    expect(screen.getByTestId("active")).toHaveTextContent("html:Brief");
  });

  it("a second showArtifact call replaces the first, not stacks", () => {
    render(<ArtifactPanelProvider><Probe /></ArtifactPanelProvider>);
    fireEvent.click(screen.getByText("show-a"));
    fireEvent.click(screen.getByText("show-b"));
    expect(screen.getByTestId("active")).toHaveTextContent("chart:Sales");
  });

  it("clearArtifact resets to none", () => {
    render(<ArtifactPanelProvider><Probe /></ArtifactPanelProvider>);
    fireEvent.click(screen.getByText("show-a"));
    fireEvent.click(screen.getByText("clear"));
    expect(screen.getByTestId("active")).toHaveTextContent("none");
  });

  it("useArtifactPanel throws when used outside a provider", () => {
    function Bare() {
      useArtifactPanel();
      return null;
    }
    expect(() => render(<Bare />)).toThrow(
      "useArtifactPanel must be used within an ArtifactPanelProvider",
    );
  });
});
