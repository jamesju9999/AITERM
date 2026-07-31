import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
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

  it("also offers the manual hint and pick-interpreter escape hatch when the install failed", () => {
    // This is the path a PyPI-blocked user actually hits: uv fetches Python
    // and builds the venv fine, so the state is "failed" (not "missing") by
    // the time `uv pip install` fails to reach PyPI — the Index URL guidance
    // has to live here too, or the one user it was built for never sees it.
    render(
      <PythonEnvGate
        state="failed"
        lines={[]}
        error="安裝 doc_core 相依套件失敗：ERROR: could not build wheel"
        onInstall={() => {}}
        onRecheck={() => {}}
        onPickInterpreter={() => {}}
      />,
    );
    expect(screen.getByRole("button", { name: /interpreter|手動指定/ })).toBeTruthy();
    expect(screen.getByText(/Index URL/)).toBeTruthy();
  });

  it("lets the user dismiss the missing/failed/broken card so it stops blocking the rest of the app", () => {
    // Without onDismiss, the card only ever closes via a successful ensure()
    // — there's no way to just get it out of the way and use another feature.
    for (const state of ["missing", "failed", "broken"] as const) {
      const onDismiss = vi.fn();
      const { unmount } = render(
        <PythonEnvGate
          state={state}
          lines={[]}
          onInstall={() => {}}
          onRecheck={() => {}}
          onDismiss={onDismiss}
        />,
      );
      fireEvent.click(screen.getByRole("button", { name: /Dismiss|關閉/ }));
      expect(onDismiss).toHaveBeenCalledOnce();
      unmount();
    }
  });

  it("does not offer a dismiss button while installing, even if onDismiss is provided", () => {
    // Closing the card mid-install would read as "cancel the install", which
    // it doesn't actually do.
    render(
      <PythonEnvGate
        state="installing"
        lines={[]}
        onInstall={() => {}}
        onRecheck={() => {}}
        onDismiss={() => {}}
      />,
    );
    expect(screen.queryByRole("button", { name: /Dismiss|關閉/ })).toBeNull();
  });

  it("does not offer a dismiss button when the caller doesn't pass onDismiss", () => {
    render(
      <PythonEnvGate state="missing" lines={[]} onInstall={() => {}} onRecheck={() => {}} />,
    );
    expect(screen.queryByRole("button", { name: /Dismiss|關閉/ })).toBeNull();
  });

  it("renders nothing once the environment is ready", () => {
    const { container } = render(
      <PythonEnvGate state="ready" lines={[]} onInstall={() => {}} onRecheck={() => {}} />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("keeps its log scrolled without touching the page", () => {
    // This card is in normal flow, so scrollIntoView would walk up to the
    // document and scroll the entire app — during an install that fires once
    // per log line, and the app stayed scrolled up after the card unmounted.
    // Spy on HTMLElement.prototype, not Element.prototype: src/test-setup.ts
    // already stubs it there, and that stub shadows anything placed further up
    // the chain — a spy on Element.prototype would never see the call.
    const scrollIntoView = vi.fn();
    const original = window.HTMLElement.prototype.scrollIntoView;
    window.HTMLElement.prototype.scrollIntoView = scrollIntoView;

    try {
      render(
        <PythonEnvGate
          state="installing"
          lines={[
            { text: "Resolved 36 packages", isError: false },
            { text: "Prepared 36 packages", isError: false },
          ]}
          onInstall={() => {}}
          onRecheck={() => {}}
        />,
      );

      expect(scrollIntoView).not.toHaveBeenCalled();
    } finally {
      window.HTMLElement.prototype.scrollIntoView = original;
    }
  });
});
