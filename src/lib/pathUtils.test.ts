import { describe, it, expect } from "vitest";
import { isPathInside } from "./pathUtils";

describe("isPathInside", () => {
  it("accepts direct child", () => {
    expect(isPathInside("/home/user/proj/src/a.ts", "/home/user/proj")).toBe(true);
  });
  it("accepts root itself", () => {
    expect(isPathInside("/home/user/proj", "/home/user/proj")).toBe(true);
  });
  it("rejects sibling", () => {
    expect(isPathInside("/home/user/other/a.ts", "/home/user/proj")).toBe(false);
  });
  it("rejects prefix-only match", () => {
    expect(isPathInside("/home/user/proj2/a.ts", "/home/user/proj")).toBe(false);
  });
  it("resolves .. escape", () => {
    expect(isPathInside("/home/user/proj/../secrets.txt", "/home/user/proj")).toBe(false);
  });
  it("resolves .. staying inside", () => {
    expect(isPathInside("/home/user/proj/src/../a.ts", "/home/user/proj")).toBe(true);
  });
  it("handles windows backslash + case-insensitive drive paths", () => {
    expect(isPathInside("C:\\Proj\\src\\a.ts", "c:\\proj")).toBe(true);
    expect(isPathInside("C:\\Other\\a.ts", "C:\\Proj")).toBe(false);
  });
  it("trailing slash on root", () => {
    expect(isPathInside("/home/user/proj/a.ts", "/home/user/proj/")).toBe(true);
  });
});
