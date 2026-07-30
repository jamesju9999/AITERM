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

  it("still shows the error and log lines from a failed first attempt when Python is missing", () => {
    // A first-run failure (e.g. no network, or uv rejecting a corrupt
    // download) must not be swallowed just because the resulting state is
    // "missing" rather than "failed" — that error is the only clue the user
    // gets about what actually went wrong.
    render(
      <PythonEnvGate
        state="missing"
        lines={[{ text: "Resolved 36 packages", isError: false }, { text: "error: connection reset", isError: true }]}
        error="無法取得 Python：network unreachable"
        onInstall={() => {}}
        onRecheck={() => {}}
      />,
    );
    expect(screen.getByText(/network unreachable/)).toBeTruthy();
    expect(screen.getByText("Resolved 36 packages")).toBeTruthy();
    expect(screen.getByText("error: connection reset")).toBeTruthy();
  });

  it("shows the broken-install message and error, but no install button, when uv itself is unusable", () => {
    render(
      <PythonEnvGate
        state="broken"
        lines={[]}
        error="uv 無法執行：permission denied"
        onInstall={() => {}}
        onRecheck={() => {}}
      />,
    );
    expect(screen.getByText(/permission denied/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Install it for me|幫我安裝/ })).toBeNull();
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
