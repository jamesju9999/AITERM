import { describe, expect, it } from "vitest";

import type { TaskRow } from "../../ipc/tasks";
import { waitingReason } from "./waitingReason";

const card = (id: string, o: Partial<TaskRow> = {}): TaskRow =>
  ({ id, status: "queued", outcome: null, depends_on: null, sort_order: 1, agent_id: "me", card_kind: "work", ...o }) as unknown as TaskRow;
const dep = (id: string, o: Partial<TaskRow> = {}): TaskRow => card(id, { agent_id: "other", ...o });

describe("waitingReason", () => {
  it("is deps when every queued card waits for another coder's unfinished prerequisite", () => {
    const d = dep("d1", { status: "running" });
    const q = card("q1", { depends_on: JSON.stringify(["d1"]) });
    expect(waitingReason([q], [d, q], true)).toBe("deps");
  });

  it("is slot when some queued card has no unmet prerequisite (only the global limit holds it back)", () => {
    const d = dep("d1", { status: "running" });
    const waiting = card("q1", { depends_on: JSON.stringify(["d1"]) });
    const ready = card("q2");
    expect(waitingReason([waiting, ready], [d, waiting, ready], true)).toBe("slot");
  });

  it("is slot for cards without prerequisites", () => {
    const q = card("q1");
    expect(waitingReason([q], [q], true)).toBe("slot");
  });

  it("is slot when the prerequisite belongs to the same coder (queue order decides, not a wait)", () => {
    const d = card("d1", { status: "running" });
    const q = card("q1", { depends_on: JSON.stringify(["d1"]) });
    expect(waitingReason([q], [d, q], true)).toBe("slot");
  });

  it("is slot for nothing queued", () => {
    expect(waitingReason([], [], true)).toBe("slot");
  });
});
