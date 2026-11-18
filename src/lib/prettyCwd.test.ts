import { describe, it, expect } from "vitest";
import { tildePath, blockCwdLabel } from "./prettyCwd";

describe("tildePath", () => {
  it("macOS/Linux: home becomes ~, subfolders become ~/…", () => {
    expect(tildePath("/Users/jamesju", "/Users/jamesju")).toBe("~");
    expect(tildePath("/Users/jamesju/Documents/GitHub", "/Users/jamesju")).toBe("~/Documents/GitHub");
  });
  it("does not treat a sibling that merely shares the prefix as inside home", () => {
    expect(tildePath("/Users/jamesju2/x", "/Users/jamesju")).toBe("/Users/jamesju2/x");
  });
  it("Windows: backslashes are normalised and the drive comparison ignores case", () => {
    expect(tildePath("C:\\Users\\jamesju", "C:\\Users\\jamesju")).toBe("~");
    expect(tildePath("c:\\users\\jamesju\\proj\\app", "C:\\Users\\jamesju")).toBe("~/proj/app");
    expect(tildePath("D:\\work", "C:\\Users\\jamesju")).toBe("D:/work");
  });
  it("without a home it just normalises", () => {
    expect(tildePath("C:\\a\\b", null)).toBe("C:/a/b");
    expect(tildePath("/a/b", "")).toBe("/a/b");
  });
});

describe("blockCwdLabel", () => {
  it("home and below show as ~ (the original complaint: '.../Users/jamesju' vs '~')", () => {
    expect(blockCwdLabel("/Users/jamesju", "/Users/jamesju")).toBe("~");
    expect(blockCwdLabel("/Users/jamesju/Documents/GitHub/AITERM", "/Users/jamesju")).toBe("~/Documents/GitHub/AITERM");
    expect(blockCwdLabel("C:\\Users\\jamesju\\proj", "C:\\Users\\jamesju")).toBe("~/proj");
  });
  it("outside home keeps the last two segments, on every platform", () => {
    expect(blockCwdLabel("/var/log/nginx", "/Users/jamesju")).toBe(".../log/nginx");
    expect(blockCwdLabel("D:\\work\\repo\\app", "C:\\Users\\jamesju")).toBe(".../repo/app");
    expect(blockCwdLabel("/tmp", "/Users/jamesju")).toBe("/tmp");
  });
  it("before the home dir is known it falls back to the shortened path; empty stays empty", () => {
    expect(blockCwdLabel("/Users/jamesju", null)).toBe(".../Users/jamesju");
    expect(blockCwdLabel(undefined, "/x")).toBe("");
  });
});
