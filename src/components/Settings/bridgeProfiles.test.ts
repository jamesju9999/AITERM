import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  loadBridgeProfiles,
  saveBridgeProfiles,
  tiersEqual,
  type BridgeProfile,
} from "./bridgeProfiles";

const PROFILE: BridgeProfile = {
  id: "p1",
  name: "個人帳號",
  opus: { provider_id: "acct-a", model: "claude-opus-4" },
  sonnet: { provider_id: "acct-a", model: "claude-sonnet-4" },
  haiku: null,
};

beforeEach(() => {
  localStorage.clear();
});

describe("loadBridgeProfiles / saveBridgeProfiles", () => {
  it("尚未存過時回傳空陣列", () => {
    expect(loadBridgeProfiles()).toEqual([]);
  });

  it("存了之後讀得回一模一樣的內容", () => {
    saveBridgeProfiles([PROFILE]);
    expect(loadBridgeProfiles()).toEqual([PROFILE]);
  });

  it("localStorage 內容是壞掉的 JSON 時回傳空陣列而不是拋例外", () => {
    localStorage.setItem("aiterm.bridgeProfiles", "{not json");
    expect(loadBridgeProfiles()).toEqual([]);
  });

  it("localStorage 內容不是陣列時回傳空陣列", () => {
    localStorage.setItem("aiterm.bridgeProfiles", JSON.stringify({ not: "an array" }));
    expect(loadBridgeProfiles()).toEqual([]);
  });

  it("讀取拋例外時回傳空陣列而不是讓呼叫端炸掉", () => {
    const spy = vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(loadBridgeProfiles()).toEqual([]);
    spy.mockRestore();
  });

  it("寫入拋例外時不拋出，呼叫端可以繼續", () => {
    const spy = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("quota exceeded");
    });
    expect(() => saveBridgeProfiles([PROFILE])).not.toThrow();
    spy.mockRestore();
  });
});

describe("tiersEqual", () => {
  it("三層都是 null 時視為相等", () => {
    expect(
      tiersEqual({ opus: null, sonnet: null, haiku: null }, { opus: null, sonnet: null, haiku: null }),
    ).toBe(true);
  });

  it("provider_id 與 model 都相同時視為相等", () => {
    const a = { opus: { provider_id: "x", model: "m" }, sonnet: null, haiku: null };
    const b = { opus: { provider_id: "x", model: "m" }, sonnet: null, haiku: null };
    expect(tiersEqual(a, b)).toBe(true);
  });

  it("model 不同就不相等", () => {
    const a = { opus: { provider_id: "x", model: "m1" }, sonnet: null, haiku: null };
    const b = { opus: { provider_id: "x", model: "m2" }, sonnet: null, haiku: null };
    expect(tiersEqual(a, b)).toBe(false);
  });

  it("一邊 null 一邊不是 null 就不相等", () => {
    const a = { opus: { provider_id: "x", model: "m" }, sonnet: null, haiku: null };
    const b = { opus: null, sonnet: null, haiku: null };
    expect(tiersEqual(a, b)).toBe(false);
  });
});
