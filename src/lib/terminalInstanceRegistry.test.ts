import { describe, expect, it } from "vitest";
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
});
