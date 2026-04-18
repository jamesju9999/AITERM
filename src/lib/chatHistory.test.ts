import { describe, expect, it } from "vitest";
import { truncateHistory } from "./chatHistory";
import type { ChatMessage } from "../ipc/ai";

function msg(role: "user" | "assistant", i: number): ChatMessage {
  return { role, content: `m${i}` };
}

describe("truncateHistory", () => {
  it("returns empty for empty input", () => {
    expect(truncateHistory([], 20)).toEqual([]);
  });

  it("returns input unchanged when shorter than limit", () => {
    const msgs = [msg("user", 1), msg("assistant", 2)];
    expect(truncateHistory(msgs, 20)).toEqual(msgs);
  });

  it("returns input unchanged when exactly at limit", () => {
    const msgs = Array.from({ length: 20 }, (_, i) =>
      msg(i % 2 === 0 ? "user" : "assistant", i),
    );
    expect(truncateHistory(msgs, 20)).toEqual(msgs);
  });

  it("keeps only the last N when over limit", () => {
    const msgs = Array.from({ length: 25 }, (_, i) =>
      msg(i % 2 === 0 ? "user" : "assistant", i),
    );
    const out = truncateHistory(msgs, 20);
    expect(out).toHaveLength(20);
    // First kept item should be m5 (dropped 0..4)
    expect(out[0].content).toBe("m5");
    expect(out[19].content).toBe("m24");
  });

  it("returns empty when limit is 0", () => {
    const msgs = [msg("user", 1)];
    expect(truncateHistory(msgs, 0)).toEqual([]);
  });

  it("returns empty when limit is negative (defensive)", () => {
    const msgs = [msg("user", 1)];
    expect(truncateHistory(msgs, -5)).toEqual([]);
  });
});
