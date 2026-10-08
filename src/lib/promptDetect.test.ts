import { describe, it, expect } from "vitest";
import { looksLikePrompt, lastNonEmptyLine } from "./promptDetect";

describe("looksLikePrompt", () => {
  it.each([
    "sh-3.2$ ",
    "[~] # ",
    "admin@host:~$ ",
    "user@mac ~ % ",
    "➜  proj git:(main) ✗ ",
    "PS C:\\Users\\a> ",
    "root@nas:/share# ",
    "~ ❯ ",
    "[jamesju@JAMESJUNAS ~]$ ",
    "[root@nas /share]# ",
    "(venv) user@host:~/proj$ ",
    "jamesju@host ~/proj % ",
  ])("accepts %j", (line) => {
    expect(looksLikePrompt(line)).toBe(true);
  });

  it.each([
    "",
    "   ",
    "total 48",
    "> ",
    "Password:",
    "Are you sure you want to continue connecting (yes/no)?",
    "-rw-r--r-- 1 a b 12 Oct 7 file.txt",
    "Downloading... 45%",
    "price is 5$ per unit okay",
  ])("rejects %j", (line) => {
    expect(looksLikePrompt(line)).toBe(false);
  });
});

describe("lastNonEmptyLine", () => {
  it("skips trailing blank lines and strips ANSI", () => {
    expect(lastNonEmptyLine("a\n\x1b[32mb$ \x1b[0m\n  \n")).toBe("b$ ");
  });
  it("returns empty string for empty input", () => {
    expect(lastNonEmptyLine("")).toBe("");
  });
});
