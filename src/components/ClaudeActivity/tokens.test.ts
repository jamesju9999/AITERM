import { describe, expect, it } from "vitest";
import type { ActivityNode } from "../../ipc/claudeActivity";
import { tokenStats } from "./tokens";

const n = (id: string, tokens: number | null): ActivityNode => ({
  id, parent: null, agent_type: "x", description: id, status: "done", started_ms: 0, ended_ms: 1,
  current_tool: null, summary: null, tokens, tool_count: null, last_event_ms: 0, turn: 1,
});

describe("tokenStats", () => {
  it("沒有任何 token 資料：合計 0、沒有最耗", () => {
    expect(tokenStats([n("a", null), n("b", null)])).toEqual({ total: 0, heaviestId: null, share: expect.any(Function) });
  });

  it("合計只算有 token 的節點；最耗是 token 最多的", () => {
    const s = tokenStats([n("a", 100), n("b", 300), n("c", null)]);
    expect(s.total).toBe(400);
    expect(s.heaviestId).toBe("b");
  });

  it("只有一個節點有 token 時不標最耗（沒有可比較的對象）", () => {
    expect(tokenStats([n("a", 100), n("b", null)]).heaviestId).toBeNull();
  });

  it("並列最多時取先出現的", () => {
    expect(tokenStats([n("a", 200), n("b", 200)]).heaviestId).toBe("a");
  });

  it("share 回傳佔合計的整數百分比；沒 token 或合計 0 回 null", () => {
    const s = tokenStats([n("a", 100), n("b", 300), n("c", null)]);
    expect(s.share(n("a", 100))).toBe(25);
    expect(s.share(n("b", 300))).toBe(75);
    expect(s.share(n("c", null))).toBeNull();
    expect(tokenStats([n("z", 0)]).share(n("z", 0))).toBeNull();
  });
});
