import { describe, expect, it, vi } from "vitest";
import type { Terminal } from "@xterm/xterm";
import { registerTerminal, serializeTerminal, unregisterTerminal } from "./terminalInstanceRegistry";

const fakeTerm = {} as Terminal;

describe("terminalInstanceRegistry", () => {
  it("returns null for an id that was never registered", () => {
    expect(serializeTerminal("never-registered")).toBeNull();
  });

  it("returns the addon's serialized output after registration", () => {
    const id = "tab-1";
    const fakeAddon = { serialize: () => "screen buffer content" };
    registerTerminal(id, fakeTerm, fakeAddon);
    expect(serializeTerminal(id)).toBe("screen buffer content");
  });

  it("returns null again after unregistering", () => {
    const id = "tab-2";
    const fakeAddon = { serialize: () => "some content" };
    registerTerminal(id, fakeTerm, fakeAddon);
    unregisterTerminal(id);
    expect(serializeTerminal(id)).toBeNull();
  });

  it("replaces the previous entry when registered twice for the same id", () => {
    const id = "tab-3";
    const firstAddon = { serialize: () => "first" };
    const secondAddon = { serialize: () => "second" };
    registerTerminal(id, fakeTerm, firstAddon);
    registerTerminal(id, fakeTerm, secondAddon);
    expect(serializeTerminal(id)).toBe("second");
  });

  // Regression test: @xterm/addon-serialize's serialize() with no options
  // dumps the ENTIRE scrollback buffer, not just the currently-visible
  // screen. For a TUI app like Claude Code that redraws by scrolling
  // through many intermediate frames (spinner animation, in-progress table
  // drafts, repeated prompts), that produced a transcript just as garbled
  // as the raw capture it was meant to replace — every historical frame
  // concatenated together instead of only the final settled screen. Must
  // pass `{ scrollback: 0 }` to serialize only the current viewport, i.e.
  // exactly what a human looking at the terminal right now would see.
  it("requests only the current viewport (scrollback: 0), not the full scrollback history", () => {
    const id = "tab-4";
    const serialize = vi.fn(() => "content");
    registerTerminal(id, fakeTerm, { serialize });
    serializeTerminal(id);
    expect(serialize).toHaveBeenCalledWith({ scrollback: 0 });
  });
});
