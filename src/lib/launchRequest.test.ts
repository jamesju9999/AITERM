import { describe, it, expect } from "vitest";
import { planLaunch, quoteArg } from "./launchRequest";

describe("quoteArg", () => {
  it("leaves safe POSIX words alone", () => {
    expect(quoteArg("ls", false)).toBe("ls");
    expect(quoteArg("/usr/local/bin/x-1.2", false)).toBe("/usr/local/bin/x-1.2");
  });

  it("single-quotes POSIX arguments containing spaces or shell metacharacters", () => {
    expect(quoteArg("/tmp/a b", false)).toBe("'/tmp/a b'");
    expect(quoteArg("a;rm -rf /", false)).toBe("'a;rm -rf /'");
  });

  it("escapes embedded single quotes the POSIX way", () => {
    expect(quoteArg("it's", false)).toBe("'it'\\''s'");
  });

  it("double-quotes Windows arguments that need it", () => {
    expect(quoteArg("C:\\Program Files\\x", true)).toBe('"C:\\Program Files\\x"');
    expect(quoteArg("C:\\x\\y", true)).toBe("C:\\x\\y");
  });
});

describe("planLaunch", () => {
  it("a cwd-only request just opens a tab there", () => {
    expect(planLaunch({ cwd: "/p", script: null, command: null }, false)).toEqual({
      kind: "open",
      cwd: "/p",
    });
  });

  it("a command request joins argv into one quoted command line", () => {
    expect(planLaunch({ cwd: "/p", script: null, command: ["ls", "-la", "/tmp/a b"] }, false)).toEqual({
      kind: "open",
      cwd: "/p",
      command: "ls -la '/tmp/a b'",
    });
  });

  it("a script request needs confirmation and carries the quoted path as its command", () => {
    expect(planLaunch({ cwd: "/p", script: "/p/my deploy.command", command: null }, false)).toEqual({
      kind: "confirm-script",
      cwd: "/p",
      scriptPath: "/p/my deploy.command",
      command: "'/p/my deploy.command'",
    });
  });

  it("when both an -e command and a script are present, the command wins and nothing needs confirming", () => {
    expect(planLaunch({ cwd: "/p", script: "/p/a.command", command: ["ls"] }, false)).toEqual({
      kind: "open",
      cwd: "/p",
      command: "ls",
    });
  });

  it("on Windows a script is only opened at its directory, never executed", () => {
    expect(planLaunch({ cwd: "C:\\p", script: "C:\\p\\a.sh", command: null }, true)).toEqual({
      kind: "open",
      cwd: "C:\\p",
    });
  });

  it("null cwd becomes an absent cwd, not the string 'null'", () => {
    const plan = planLaunch({ cwd: null, script: null, command: ["htop"] }, false);
    expect(plan).toEqual({ kind: "open", command: "htop" });
    expect("cwd" in plan).toBe(false);
  });
});
