import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { AgentRow } from "../../ipc/agents";
import type { TaskRow } from "../../ipc/tasks";
import { AgentNode } from "./AgentNode";

const verifier = { id: "v", kind: "verifier", name: "驗收者", role: "", prompt_prefix: "", canvas_x: 0, canvas_y: 0 } as unknown as AgentRow;
const verifyCard = (over: Partial<TaskRow>): TaskRow =>
  ({ id: "vc1", title: "驗收", card_kind: "verify", status: "done", outcome: "success", verdict: null, review_reasons: null, error_message: null, agent_id: "v", finished_at: 10, ...over }) as unknown as TaskRow;

const mount = (cards: TaskRow[]) => {
  const onVerifyReport = vi.fn();
  const onAcceptVerify = vi.fn();
  render(
    <LocaleProvider>
      <AgentNode
        agent={verifier}
        tasks={cards}
        agentName={() => "x"}
        coderIds={new Set()}
        hasReviewer={false}
        hasOrchestrator={false}
        hasVerifier
        status="idle"
        nowSecs={0}
        dropState={null}
        mergeMessage={null}
        mergeConflict={false}
        onHeaderMouseDown={() => {}}
        onChipMouseDown={() => {}}
        onEdit={() => {}}
        onDelete={() => {}}
        onRetry={() => {}}
        onSkip={() => {}}
        onTranscript={() => {}}
        onOpenTab={() => {}}
        onMerge={() => {}}
        onAbortMerge={() => {}}
        onRelease={() => {}}
        onMarkDone={() => {}}
        onVerifyReport={onVerifyReport}
        onAcceptVerify={onAcceptVerify}
      />
    </LocaleProvider>,
  );
  return { onVerifyReport, onAcceptVerify };
};

afterEach(cleanup);

describe("驗收者卡片（server 驗收的新狀態）", () => {
  it("驗收卡自己失敗（沒有結論）：顯示「驗收沒有完成」與原因，可以看報告與放行，不會被當成「退回 0 項」", () => {
    const { onVerifyReport, onAcceptVerify } = mount([verifyCard({ outcome: "failed", error_message: "合併衝突：「工程師2」的分支無法併進整合版本" })]);
    expect(screen.getByText(/驗收沒有完成/)).toBeInTheDocument();
    expect(screen.getByText(/合併衝突/)).toBeInTheDocument();
    expect(screen.queryByText(/驗收退回/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "驗收報告" }));
    expect(onVerifyReport).toHaveBeenCalledWith("vc1");
    fireEvent.click(screen.getByRole("button", { name: "放行" }));
    expect(onAcceptVerify).toHaveBeenCalledWith("vc1");
  });

  it("通過與要你決定的狀態照舊", () => {
    mount([verifyCard({ verdict: "pass" })]);
    expect(screen.getByText(/驗收通過/)).toBeInTheDocument();
  });
});
