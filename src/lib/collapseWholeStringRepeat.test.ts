import { describe, expect, it } from "vitest";
import { collapseWholeStringRepeat } from "./collapseWholeStringRepeat";

describe("collapseWholeStringRepeat", () => {
  it("collapses a string that is exactly the same chunk repeated twice", () => {
    const prompt = "jamesju@JamesdeMacBook-Pro specs % ";
    expect(collapseWholeStringRepeat(prompt + prompt)).toBe(prompt);
  });

  it("collapses a string that is exactly the same chunk repeated three times", () => {
    const prompt = "user@host ~ % ";
    expect(collapseWholeStringRepeat(prompt + prompt + prompt)).toBe(prompt);
  });

  it("leaves ordinary, non-repeated content unchanged", () => {
    const text = "hello world, this is not a repeated string";
    expect(collapseWholeStringRepeat(text)).toBe(text);
  });

  it("does not collapse two different lines that happen to have the same length", () => {
    const text = "first line here!" + "second line here";
    expect(collapseWholeStringRepeat(text)).toBe(text);
  });

  it("leaves an empty string unchanged", () => {
    expect(collapseWholeStringRepeat("")).toBe("");
  });

  it("leaves a single occurrence (no repetition) unchanged", () => {
    const prompt = "jamesju@JamesdeMacBook-Pro specs % ";
    expect(collapseWholeStringRepeat(prompt)).toBe(prompt);
  });

  it("does not collapse a partial repeat (trailing extra content breaks the pattern)", () => {
    const prompt = "user@host ~ % ";
    expect(collapseWholeStringRepeat(prompt + prompt + "extra")).toBe(prompt + prompt + "extra");
  });

  it("does not collapse real command output that happens to contain a repeated substring mid-string", () => {
    const text = "abcabc def";
    expect(collapseWholeStringRepeat(text)).toBe(text);
  });
});
