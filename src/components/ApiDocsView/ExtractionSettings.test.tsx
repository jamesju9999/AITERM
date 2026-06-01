import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ExtractionSettings } from "./ExtractionSettings";
import type { KeepOptions, AuthStatus } from "../../ipc/apiDocs";
import { LocaleProvider } from "../../contexts/LocaleContext";

beforeEach(() => {
  const localStorageMock = {
    getItem: vi.fn((key: string) => (key === "aiterm_locale" ? "en" : null)),
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

const defaultKeep: KeepOptions = {
  description: true,
  parameters: true,
  request_body: true,
  responses: true,
  code_samples: true,
};

const notLoggedIn: AuthStatus = { logged_in: false, account: "" };

const wrap = (ui: React.ReactNode) =>
  render(<LocaleProvider>{ui}</LocaleProvider>);

describe("ExtractionSettings", () => {
  it("renders output dir input", () => {
    wrap(
      <ExtractionSettings
        outputDir=""
        onOutputDirChange={() => {}}
        merge={false}
        onMergeChange={() => {}}
        keep={defaultKeep}
        onKeepChange={() => {}}
        auth={notLoggedIn}
        domain=""
        onLogin={() => {}}
        onLogout={() => {}}
        extracting={false}
        selectedCount={0}
        hasProvider={true}
        onExtractRaw={() => {}}
        onExtractAi={() => {}}
      />
    );
    expect(screen.getByPlaceholderText("~/api-docs")).toBeInTheDocument();
  });

  it("disables AI enhance button when no provider", () => {
    wrap(
      <ExtractionSettings
        outputDir="/tmp"
        onOutputDirChange={() => {}}
        merge={false}
        onMergeChange={() => {}}
        keep={defaultKeep}
        onKeepChange={() => {}}
        auth={notLoggedIn}
        domain=""
        onLogin={() => {}}
        onLogout={() => {}}
        extracting={false}
        selectedCount={1}
        hasProvider={false}
        onExtractRaw={() => {}}
        onExtractAi={() => {}}
      />
    );
    const aiBtn = screen.getByText("Extract + AI Enhance");
    expect(aiBtn).toBeDisabled();
  });

  it("calls onLogin when login button clicked", () => {
    const onLogin = vi.fn();
    wrap(
      <ExtractionSettings
        outputDir=""
        onOutputDirChange={() => {}}
        merge={false}
        onMergeChange={() => {}}
        keep={defaultKeep}
        onKeepChange={() => {}}
        auth={notLoggedIn}
        domain="docs.example.com"
        onLogin={onLogin}
        onLogout={() => {}}
        extracting={false}
        selectedCount={0}
        hasProvider={true}
        onExtractRaw={() => {}}
        onExtractAi={() => {}}
      />
    );
    fireEvent.click(screen.getByText("Login"));
    expect(onLogin).toHaveBeenCalled();
  });
});
