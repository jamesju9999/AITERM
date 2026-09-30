import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { TranscriptDialog } from "./TranscriptDialog";

const readTranscript = vi.fn();
vi.mock("../../ipc/tasks", () => ({
  readTranscript: (...a: unknown[]) => readTranscript(...a),
}));

const TEXT = ["❯ first prompt", "answer one", "❯ second prompt", "answer two"].join("\n");

async function mount(text: string) {
  readTranscript.mockResolvedValue(text);
  render(
    <LocaleProvider>
      <TranscriptDialog projectId="p" taskId="t" body="body" onClose={() => {}} />
    </LocaleProvider>,
  );
  await screen.findByTestId("task-transcript-turns").catch(() => screen.findByTestId("task-transcript-raw"));
}

describe("TranscriptDialog", () => {
  beforeEach(() => {
    localStorage.clear();
    readTranscript.mockReset();
  });

  it("lists prompts with output collapsed by default", async () => {
    await mount(TEXT);
    expect(screen.getByTestId("task-transcript-turns")).toBeTruthy();
    expect(screen.getByText(/first prompt/)).toBeTruthy();
    expect(screen.getByText(/second prompt/)).toBeTruthy();
    expect(screen.queryByText("answer one")).toBeNull();
  });

  it("expands a single turn on click", async () => {
    await mount(TEXT);
    fireEvent.click(screen.getByText(/first prompt/));
    expect(screen.getByText("answer one")).toBeTruthy();
    expect(screen.queryByText("answer two")).toBeNull();
  });

  it("expand all / collapse all", async () => {
    await mount(TEXT);
    fireEvent.click(screen.getByText("全部展開"));
    expect(screen.getByText("answer one")).toBeTruthy();
    expect(screen.getByText("answer two")).toBeTruthy();
    fireEvent.click(screen.getByText("全部收合"));
    expect(screen.queryByText("answer one")).toBeNull();
  });

  it("raw toggle shows the original text", async () => {
    await mount(TEXT);
    fireEvent.click(screen.getByText("原始文字"));
    expect(screen.getByTestId("task-transcript-raw").textContent).toBe(TEXT);
  });

  it("falls back to raw text without controls when no prompts exist", async () => {
    await mount("plain output only");
    expect(screen.getByTestId("task-transcript-raw").textContent).toBe("plain output only");
    expect(screen.queryByText("全部展開")).toBeNull();
    expect(screen.queryByText("原始文字")).toBeNull();
  });
});
