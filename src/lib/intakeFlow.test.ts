import { describe, expect, it } from "vitest";
import { flowSteps } from "./intakeFlow";

const states = (o: Parameters<typeof flowSteps>[0]) => flowSteps(o).map((s) => s.state).join(",");
const base = { kind: null, verified: null, verification: null, error: null } as const;

describe("flowSteps", () => {
  it("walks forward as the item progresses", () => {
    expect(states({ ...base, status: "pending" })).toBe("done,current,todo,todo,todo,todo");
    expect(states({ ...base, status: "queued" })).toBe("done,done,current,todo,todo,todo");
    expect(states({ ...base, status: "running" })).toBe("done,done,done,current,todo,todo");
    expect(states({ ...base, status: "draft", kind: "question" })).toBe("done,done,done,done,skipped,current");
    expect(states({ ...base, status: "done", kind: "question" })).toBe("done,done,done,done,skipped,done");
  });

  it("an item on the board is being answered, with the run step already done", () => {
    expect(states({ ...base, status: "on_board", kind: "code_change" })).toBe("done,done,done,current,todo,todo");
  });

  it("a board item that was never verified here shows the verify step as skipped, not done", () => {
    expect(states({ ...base, status: "draft", kind: "code_change", board_task_id: "card" })).toBe("done,done,done,done,skipped,current");
    expect(states({ ...base, status: "done", kind: "code_change", board_task_id: "card" })).toBe("done,done,done,done,skipped,done");
  });

  it("a code change is verified; a failed verification shows as failed", () => {
    expect(states({ ...base, status: "done", kind: "code_change", verified: true, verification: "ok" })).toBe("done,done,done,done,done,done");
    expect(states({ ...base, status: "draft", kind: "code_change", verified: false, verification: "紅" })).toBe("done,done,done,done,failed,current");
  });

  it("a rejected item stops at approval; a draft whose send failed marks the last step", () => {
    expect(states({ ...base, status: "rejected" })).toBe("done,failed,todo,todo,todo,todo");
    expect(states({ ...base, status: "draft", kind: "question", error: "SMTP" })).toBe("done,done,done,done,skipped,failed");
  });
});
