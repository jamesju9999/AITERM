import { describe, expect, it } from "vitest";
import { groupByLabel } from "./groupByLabel";
import type { TaskWithAttachments } from "../../ipc/tasks";

const card = (over: Partial<TaskWithAttachments>): TaskWithAttachments => ({
  id: "c1", title: "Card", body: "", project_dir: "/r", status: "planning",
  parallel_ok: true, interactive: false, sort_order: 1, outcome: null, tab_id: null,
  transcript_path: null, error_message: null, created_at: "2026-01-01 00:00:00",
  dispatched_at: null, finished_at: null, ai_summary: null, archived_at: null,
  session_id: null, session_path: null, use_bridge: false, bridge_tiers: null,
  label: null, attachments: [],
  ...over,
});

describe("groupByLabel", () => {
  it("沒有 label 的卡片全部進 ungrouped，順序不變", () => {
    const cards = [card({ id: "a" }), card({ id: "b" })];
    const { ungrouped, groups } = groupByLabel(cards);
    expect(ungrouped.map((c) => c.id)).toEqual(["a", "b"]);
    expect(groups).toEqual([]);
  });

  it("依 label 分組，group 內順序沿用輸入順序", () => {
    const cards = [
      card({ id: "a", label: "緊急", created_at: "2026-01-01 00:00:00" }),
      card({ id: "b", label: "文件", created_at: "2026-01-02 00:00:00" }),
      card({ id: "c", label: "緊急", created_at: "2026-01-03 00:00:00" }),
    ];
    const { ungrouped, groups } = groupByLabel(cards);
    expect(ungrouped).toEqual([]);
    expect(groups.map((g) => g.label)).toEqual(["緊急", "文件"]);
    expect(groups[0].cards.map((c) => c.id)).toEqual(["a", "c"]);
  });

  it("群組順序依該 label 最早出現的 created_at，不是依數量或字母", () => {
    const cards = [
      // "文件" 只有一張，但比 "緊急" 早出現，應該排在前面。
      card({ id: "a", label: "文件", created_at: "2026-01-01 00:00:00" }),
      card({ id: "b", label: "緊急", created_at: "2026-01-02 00:00:00" }),
      card({ id: "c", label: "緊急", created_at: "2026-01-03 00:00:00" }),
    ];
    const { groups } = groupByLabel(cards);
    expect(groups.map((g) => g.label)).toEqual(["文件", "緊急"]);
  });

  it("空白字串跟只有空白的 label 都當作未分類", () => {
    const cards = [card({ id: "a", label: "" }), card({ id: "b", label: "   " })];
    const { ungrouped, groups } = groupByLabel(cards);
    expect(ungrouped.map((c) => c.id)).toEqual(["a", "b"]);
    expect(groups).toEqual([]);
  });

  it("label 前後空白會被 trim 之後當同一組", () => {
    const cards = [card({ id: "a", label: "緊急" }), card({ id: "b", label: " 緊急 " })];
    const { groups } = groupByLabel(cards);
    expect(groups).toHaveLength(1);
    expect(groups[0].cards.map((c) => c.id)).toEqual(["a", "b"]);
  });
});
