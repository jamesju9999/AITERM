import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { isClaudeActivityEnabled, setClaudeActivityEnabled, useClaudeActivityEnabled } from "./settings";

describe("claude activity enabled setting", () => {
  beforeEach(() => localStorage.clear());

  it("defaults to enabled", () => {
    expect(isClaudeActivityEnabled()).toBe(true);
  });

  it("persists off and on", () => {
    setClaudeActivityEnabled(false);
    expect(isClaudeActivityEnabled()).toBe(false);
    setClaudeActivityEnabled(true);
    expect(isClaudeActivityEnabled()).toBe(true);
  });

  it("hook follows changes", () => {
    const { result } = renderHook(() => useClaudeActivityEnabled());
    expect(result.current).toBe(true);
    act(() => setClaudeActivityEnabled(false));
    expect(result.current).toBe(false);
  });
});
