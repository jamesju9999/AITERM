import { describe, it, expect } from "vitest";
import { planLaunch, quoteArg } from "./launchRequest";

describe("quoteArg", () => {
  it("leaves safe POSIX words alone", () => {
    expect(quoteArg("ls")).toBe("ls");
    expect(quoteArg("/usr/local/bin/x-1.2")).toBe("/usr/local/bin/x-1.2");
  });

  it("single-quotes POSIX arguments containing spaces or shell metacharacters", () => {
    expect(quoteArg("/tmp/a b")).toBe("'/tmp/a b'");
    expect(quoteArg("a;rm -rf /")).toBe("'a;rm -rf /'");
  });

  it("escapes embedded single quotes the POSIX way", () => {
    expect(quoteArg("it's")).toBe("'it'\\''s'");
  });

  it("quotes = so zsh cannot treat it as command-path expansion or an env assignment", () => {
    expect(quoteArg("=ls")).toBe("'=ls'");
    expect(quoteArg("--opt=val")).toBe("'--opt=val'");
  });

  it("quotes % so zsh cannot treat a leading one as a job reference", () => {
    expect(quoteArg("%1")).toBe("'%1'");
  });

  it("quotes the empty string so it survives as an argument", () => {
    expect(quoteArg("")).toBe("''");
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

  it("a leading VAR=value word is quoted so it is not run as an env assignment", () => {
    expect(planLaunch({ cwd: "/p", script: null, command: ["FOO=bar", "cmd"] }, false)).toEqual({
      kind: "open",
      cwd: "/p",
      command: "'FOO=bar' cmd",
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

  it("an empty command array falls through to the script, or to a plain open", () => {
    expect(planLaunch({ cwd: "/p", script: "/p/a.command", command: [] }, false)).toEqual({
      kind: "confirm-script",
      cwd: "/p",
      scriptPath: "/p/a.command",
      command: "/p/a.command",
    });
    expect(planLaunch({ cwd: "/p", script: null, command: [] }, false)).toEqual({
      kind: "open",
      cwd: "/p",
    });
  });

  it("null cwd becomes an absent cwd, not the string 'null'", () => {
    const plan = planLaunch({ cwd: null, script: null, command: ["htop"] }, false);
    expect(plan).toEqual({ kind: "open", command: "htop" });
    expect("cwd" in plan).toBe(false);
  });
});

describe("planLaunch control characters", () => {
  it("never sends a command containing a tab (the line editor would run completion)", () => {
    expect(planLaunch({ cwd: "/p", script: null, command: ["echo", "a\tb"] }, false)).toEqual({
      kind: "open",
      cwd: "/p",
    });
  });

  it("never sends a command containing ^C followed by a second command line", () => {
    expect(planLaunch({ cwd: "/p", script: null, command: ["echo", "\x03echo INJECTED\r"] }, false)).toEqual({
      kind: "open",
      cwd: "/p",
    });
  });

  it("checks every argument, not just the first, and also catches newline, DEL and NUL", () => {
    for (const bad of ["a\nb", "a\rb", "a\x7fb", "a\x00b", "a\x1bb"]) {
      expect(planLaunch({ cwd: "/p", script: null, command: ["ls", "ok", bad] }, false)).toEqual({
        kind: "open",
        cwd: "/p",
      });
    }
  });

  it("does not ask to confirm a script whose path contains a control character", () => {
    const plan = planLaunch({ cwd: "/p", script: "/p/a\tb.command", command: null }, false);
    expect(plan).toEqual({ kind: "open", cwd: "/p" });
    expect(plan.kind).not.toBe("confirm-script");
  });

  it("drops the command but still omits cwd when cwd is null", () => {
    const plan = planLaunch({ cwd: null, script: null, command: ["a\tb"] }, false);
    expect(plan).toEqual({ kind: "open" });
    expect("cwd" in plan).toBe(false);
  });

  it("positive control: a plain argument with a space still produces a command", () => {
    expect(planLaunch({ cwd: "/p", script: null, command: ["echo", "a b"] }, false)).toEqual({
      kind: "open",
      cwd: "/p",
      command: "echo 'a b'",
    });
  });
});

describe("planLaunch on Windows", () => {
  it("only opens at the directory, never executes a script", () => {
    expect(planLaunch({ cwd: "C:\\p", script: "C:\\p\\a.sh", command: null }, true)).toEqual({
      kind: "open",
      cwd: "C:\\p",
    });
  });

  it("drops an -e command instead of sending it", () => {
    expect(planLaunch({ cwd: "C:\\p", script: null, command: ["dir", "C:\\Program Files"] }, true)).toEqual({
      kind: "open",
      cwd: "C:\\p",
    });
  });

  it("drops the command even when a script is also present, and omits a null cwd", () => {
    const plan = planLaunch({ cwd: null, script: "C:\\p\\a.sh", command: ["dir"] }, true);
    expect(plan).toEqual({ kind: "open" });
    expect("cwd" in plan).toBe(false);
  });
});
