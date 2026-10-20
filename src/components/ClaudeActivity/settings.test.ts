import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import {
  isClaudeActivityEnabled, isClaudeActivityNotifyEnabled, setClaudeActivityEnabled,
  setClaudeActivityNotifyEnabled, useClaudeActivityEnabled, useClaudeActivityNotifyEnabled,
} from "./settings";

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

  it("notify setting defaults on, persists, and is independent of the graph switch", () => {
    expect(isClaudeActivityNotifyEnabled()).toBe(true);
    setClaudeActivityNotifyEnabled(false);
    expect(isClaudeActivityNotifyEnabled()).toBe(false);
    expect(isClaudeActivityEnabled()).toBe(true);
    setClaudeActivityEnabled(false);
    expect(isClaudeActivityNotifyEnabled()).toBe(false);
  });

  it("notify hook follows changes", () => {
    const { result } = renderHook(() => useClaudeActivityNotifyEnabled());
    expect(result.current).toBe(true);
    act(() => setClaudeActivityNotifyEnabled(false));
    expect(result.current).toBe(false);
  });
});
