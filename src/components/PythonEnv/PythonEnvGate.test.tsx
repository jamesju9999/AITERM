import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { PythonEnvGate } from "./PythonEnvGate";

describe("PythonEnvGate", () => {
  it("shows progress and the first-run hint while installing", () => {
    render(
      <PythonEnvGate
        state="installing"
        lines={[{ text: "Resolved 36 packages", isError: false }]}
        onInstall={() => {}}
        onRecheck={() => {}}
      />,
    );
    expect(screen.getByText("Resolved 36 packages")).toBeTruthy();
    // The 25s wait needs an explanation, or it reads as a freeze.
    expect(screen.getByText(/20–30|20-30/)).toBeTruthy();
  });

  it("offers all three escape hatches when Python is missing", () => {
    render(
      <PythonEnvGate
        state="missing"
        lines={[]}
        onInstall={() => {}}
        onRecheck={() => {}}
        onPickInterpreter={() => {}}
      />,
    );
    // Query by role: the manual-hint paragraph mentions the same phrase as the
    // button, and matching on text alone can't tell them apart — nor should the
    // markup be contorted to make it possible.
    expect(screen.getByRole("button", { name: /Install it for me|幫我安裝/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /check again|重新偵測/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /interpreter|手動指定/ })).toBeTruthy();
  });

  it("shows the error and a retry when the install failed", () => {
    render(
      <PythonEnvGate
        state="failed"
        lines={[]}
        error="安裝 doc_core 相依套件失敗：ERROR: could not build wheel"
        onInstall={() => {}}
        onRecheck={() => {}}
      />,
    );
    expect(screen.getByText(/could not build wheel/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /Retry|重試/ })).toBeTruthy();
  });

  it("renders nothing once the environment is ready", () => {
    const { container } = render(
      <PythonEnvGate state="ready" lines={[]} onInstall={() => {}} onRecheck={() => {}} />,
    );
    expect(container.firstChild).toBeNull();
  });
});
