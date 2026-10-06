import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Terminal } from "@xterm/xterm";

const writePty = vi.fn().mockResolvedValue(undefined);
vi.mock("../ipc/pty", () => ({ writePty: (...a: unknown[]) => writePty(...a) }));

import { registerTerminal, unregisterTerminal, fillTerminalInput, submitTerminalInput } from "./terminalInstanceRegistry";

const addon = { serialize: () => "" };
const term = (bracketed: boolean) => ({ modes: { bracketedPasteMode: bracketed } }) as unknown as Terminal;

beforeEach(() => writePty.mockClear());

describe("fillTerminalInput", () => {
  it("returns false and writes nothing for an unregistered id", async () => {
    expect(await fillTerminalInput("nope", "hi")).toBe(false);
    expect(writePty).not.toHaveBeenCalled();
  });

  it("wraps text in a bracketed paste when the program enabled it, with no Enter", async () => {
    registerTerminal("a", term(true), addon);
    expect(await fillTerminalInput("a", "line1\nline2")).toBe(true);
    expect(writePty).toHaveBeenCalledWith("a", "\x1b[200~line1\nline2\x1b[201~");
    unregisterTerminal("a");
  });

  it("without bracketed paste, flattens newlines so nothing gets executed", async () => {
    registerTerminal("b", term(false), addon);
    await fillTerminalInput("b", "line1\nline2\r\nline3");
    expect(writePty).toHaveBeenCalledWith("b", "line1 line2 line3");
    const sent = writePty.mock.calls[0][1] as string;
    expect(sent).not.toMatch(/[\r\n]/);
    unregisterTerminal("b");
  });

  it("strips embedded paste-end markers so text cannot break out of the paste", async () => {
    registerTerminal("c", term(true), addon);
    await fillTerminalInput("c", "x\x1b[201~rm -rf /\r");
    const sent = writePty.mock.calls[0][1] as string;
    // eslint-disable-next-line no-control-regex -- counting real ESC sequences is the point
    expect(sent.match(/\x1b\[201~/g)).toHaveLength(1);
    expect(sent.endsWith("\x1b[201~")).toBe(true);
    unregisterTerminal("c");
  });
});

describe("fillTerminalInput replace", () => {
  it("clears the current input line first (Ctrl+E, Ctrl+U) in the same write", async () => {
    registerTerminal("r", term(true), addon);
    await fillTerminalInput("r", "new", { replace: true });
    expect(writePty).toHaveBeenCalledTimes(1);
    expect(writePty).toHaveBeenCalledWith("r", "\x05\x15\x1b[200~new\x1b[201~");
    unregisterTerminal("r");
  });

  it("does not touch the line when replace is not requested", async () => {
    registerTerminal("n", term(true), addon);
    await fillTerminalInput("n", "new", { replace: false });
    expect(writePty).toHaveBeenCalledWith("n", "\x1b[200~new\x1b[201~");
    unregisterTerminal("n");
  });
});

describe("submitTerminalInput", () => {
  it("returns false and writes nothing for an unregistered id", async () => {
    expect(await submitTerminalInput("nope")).toBe(false);
    expect(writePty).not.toHaveBeenCalled();
  });

  it("sends exactly one Enter", async () => {
    registerTerminal("d", term(true), addon);
    expect(await submitTerminalInput("d")).toBe(true);
    expect(writePty).toHaveBeenCalledTimes(1);
    expect(writePty).toHaveBeenCalledWith("d", "\r");
    unregisterTerminal("d");
  });
});
