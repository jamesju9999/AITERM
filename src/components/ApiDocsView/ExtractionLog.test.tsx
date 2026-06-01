import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ExtractionLog } from "./ExtractionLog";
import { LocaleProvider } from "../../contexts/LocaleContext";

beforeEach(() => {
  const localStorageMock = {
    getItem: vi.fn(() => null),
    setItem: vi.fn(),
    removeItem: vi.fn(),
    clear: vi.fn(),
    key: vi.fn(),
    length: 0,
  };
  Object.defineProperty(window, "localStorage", {
    value: localStorageMock,
    writable: true,
  });
});

describe("ExtractionLog", () => {
  it("renders progress bar correctly", () => {
    render(
      <LocaleProvider>
        <ExtractionLog
          current={3}
          total={10}
          logs={[]}
          outputFiles={[]}
        />
      </LocaleProvider>
    );
    const bar = screen.getByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "3");
    expect(bar).toHaveAttribute("aria-valuemax", "10");
  });

  it("renders log messages with correct level class", () => {
    render(
      <LocaleProvider>
        <ExtractionLog
          current={1}
          total={5}
          logs={[
            { level: "info", message: "✓ Loaded" },
            { level: "error", message: "✗ Failed" },
          ]}
          outputFiles={[]}
        />
      </LocaleProvider>
    );
    expect(screen.getByText("✓ Loaded")).toBeInTheDocument();
    expect(screen.getByText("✗ Failed")).toBeInTheDocument();
  });

  it("renders output files when done", () => {
    render(
      <LocaleProvider>
        <ExtractionLog
          current={5}
          total={5}
          logs={[]}
          outputFiles={["/tmp/api.md", "/tmp/payments.md"]}
        />
      </LocaleProvider>
    );
    expect(screen.getByText("/tmp/api.md")).toBeInTheDocument();
  });
});
