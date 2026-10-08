import { beforeEach, describe, expect, it } from "vitest";
import { bookmarkCommand, loadBookmarks } from "./CommandBookmarks";

beforeEach(() => localStorage.clear());

describe("bookmarkCommand", () => {
  it("新指令回傳 true 並存入", () => {
    expect(bookmarkCommand("ls -la")).toBe(true);
    expect(loadBookmarks().map((b) => b.command)).toEqual(["ls -la"]);
  });

  it("同一個指令（含前後空白差異）不會重複存，回傳 false", () => {
    expect(bookmarkCommand("ls -la")).toBe(true);
    expect(bookmarkCommand("  ls -la ")).toBe(false);
    expect(loadBookmarks()).toHaveLength(1);
  });

  it("不同指令各自存入", () => {
    bookmarkCommand("ls");
    expect(bookmarkCommand("pwd")).toBe(true);
    expect(loadBookmarks()).toHaveLength(2);
  });
});
