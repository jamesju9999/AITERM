import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import { MethodDiscussion } from "./MethodDiscussion";

const json = (o: Partial<Record<string, string>> = {}) =>
  JSON.stringify({ approach: "batch", reason: "骨架相同", trial: "先試 5 支", requirement: "轉換 Cm 畫面", ...o });

interface Opts {
  start?: (opening: string) => Promise<{ discussionId: string }>;
  read?: (id: string) => Promise<string | null>;
  confirmLocked?: boolean;
}

function setup({ start, read, confirmLocked = false }: Opts = {}) {
  const startFn = vi.fn(start ?? (async () => ({ discussionId: "d1" })));
  const readFn = vi.fn(read ?? (async () => null));
  const onConfirm = vi.fn();
  const onBack = vi.fn();
  const view = render(
    <LocaleProvider>
      <MethodDiscussion
        start={startFn}
        read={readFn}
        busy={false}
        confirmLocked={confirmLocked}
        onConfirm={onConfirm}
        onBack={onBack}
        pollMs={10}
      />
    </LocaleProvider>,
  );
  return { startFn, readFn, onConfirm, onBack, ...view };
}

const begin = async (text = "533 支畫面") => {
  fireEvent.change(screen.getByTestId("ac-md-input"), { target: { value: text } });
  fireEvent.click(screen.getByTestId("ac-md-start"));
};

describe("MethodDiscussion", () => {
  it("disables start while the opening is blank", () => {
    setup();
    const btn = screen.getByTestId("ac-md-start") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    fireEvent.change(screen.getByTestId("ac-md-input"), { target: { value: "hi" } });
    expect(btn.disabled).toBe(false);
  });

  it("calls start with the opening and enters running", async () => {
    const { startFn } = setup();
    await begin("  533 支畫面 ");
    await waitFor(() => expect(startFn).toHaveBeenCalledWith("533 支畫面"));
    await waitFor(() => expect(screen.queryByTestId("ac-md-input")).toBeNull());
    expect(screen.getByTestId("ac-md-status")).toBeTruthy();
  });

  it("shows the decision card once read returns valid JSON, and confirm works", async () => {
    let n = 0;
    const { onConfirm, readFn } = setup({ read: async () => (++n < 3 ? null : json()) });
    await begin();
    await screen.findByTestId("ac-md-decision");
    expect(readFn).toHaveBeenCalledWith("d1");
    const confirm = screen.getByTestId("ac-md-confirm") as HTMLButtonElement;
    expect(confirm.disabled).toBe(false);
    fireEvent.click(confirm);
    expect(onConfirm).toHaveBeenCalledWith(expect.objectContaining({ approach: "batch", requirement: "轉換 Cm 畫面" }));
  });

  it("shows no decision card for malformed JSON", async () => {
    const { readFn } = setup({ read: async () => "{not json" });
    await begin();
    await waitFor(() => expect(readFn.mock.calls.length).toBeGreaterThan(2));
    expect(screen.queryByTestId("ac-md-decision")).toBeNull();
  });

  it("keeps polling after the card appears and updates it", async () => {
    let text = json();
    setup({ read: async () => text });
    await begin();
    await waitFor(() => expect(screen.getByTestId("ac-md-decision").textContent).toContain("骨架相同"));
    text = json({ reason: "改成新理由" });
    await waitFor(() => expect(screen.getByTestId("ac-md-decision").textContent).toContain("改成新理由"));
  });

  it("keeps the card but disables confirm when confirmLocked", async () => {
    setup({ read: async () => json(), confirmLocked: true });
    await begin();
    await screen.findByTestId("ac-md-decision");
    expect((screen.getByTestId("ac-md-confirm") as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows the error and stays idle when start fails", async () => {
    const { readFn } = setup({ start: async () => Promise.reject(new Error("boom")) });
    await begin();
    await waitFor(() => expect(screen.getByText(/boom/)).toBeTruthy());
    expect(screen.getByTestId("ac-md-input")).toBeTruthy();
    expect(readFn).not.toHaveBeenCalled();
  });

  it("stops reading after unmount", async () => {
    const { readFn, unmount } = setup();
    await begin();
    await waitFor(() => expect(readFn).toHaveBeenCalled());
    unmount();
    const before = readFn.mock.calls.length;
    await act(async () => {
      await new Promise((r) => setTimeout(r, 60));
    });
    expect(readFn.mock.calls.length).toBe(before);
  });

  it("restart clears the card and stops polling", async () => {
    const { readFn } = setup({ read: async () => json() });
    await begin();
    await screen.findByTestId("ac-md-decision");
    fireEvent.click(screen.getByTestId("ac-md-restart"));
    expect(screen.queryByTestId("ac-md-decision")).toBeNull();
    expect(screen.getByTestId("ac-md-input")).toBeTruthy();
    await act(async () => {
      await new Promise((r) => setTimeout(r, 30));
    });
    const before = readFn.mock.calls.length;
    await act(async () => {
      await new Promise((r) => setTimeout(r, 60));
    });
    expect(readFn.mock.calls.length).toBe(before);
    expect(screen.queryByTestId("ac-md-decision")).toBeNull();
  });

  it("the back button calls onBack", () => {
    const { onBack } = setup();
    fireEvent.click(screen.getByTestId("ac-md-back"));
    expect(onBack).toHaveBeenCalled();
  });
});
