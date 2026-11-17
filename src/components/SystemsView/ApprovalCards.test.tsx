import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";
import type { PendingApproval } from "../../ipc/systems";
import { translations } from "../../lib/i18n";
import { ApprovalCards } from "./ApprovalCards";

const t = translations["zh-TW"];
const a = (over: Partial<PendingApproval> = {}): PendingApproval => ({
  id: "a1",
  system_id: "s1",
  intent: "寫：送出訂單 A-102",
  method: "POST",
  url: "https://x/orders",
  declared_read: false,
  ts_ms: 1,
  ...over,
});

function mount(list: PendingApproval[], handlers = { onDecide: vi.fn(), onRejectAndDisable: vi.fn() }) {
  render(
    <LocaleProvider>
      <ApprovalCards approvals={list} nameOf={(id) => `名稱-${id}`} {...handlers} />
    </LocaleProvider>,
  );
  return handlers;
}

describe("ApprovalCards", () => {
  it("shows the AI's intent next to the actual request", () => {
    mount([a()]);
    expect(screen.getByText("寫：送出訂單 A-102")).toBeTruthy();
    expect(screen.getByText("POST https://x/orders")).toBeTruthy();
    expect(screen.getByText("名稱-s1")).toBeTruthy();
    expect(screen.queryByTestId("declared-read-warning")).toBeNull();
  });

  it("warns when the AI declared a read but triggered a write", () => {
    mount([a({ declared_read: true })]);
    expect(screen.getByTestId("declared-read-warning")).toBeTruthy();
  });

  it("reports approve, reject and reject-and-disable on the right card", () => {
    const h = mount([a(), a({ id: "a2", system_id: "s2" })]);
    const cards = screen.getAllByTestId("approval-card");
    fireEvent.click(cards[0].querySelectorAll("button")[0]);
    fireEvent.click(cards[1].querySelectorAll("button")[1]);
    fireEvent.click(cards[1].querySelectorAll("button")[2]);
    expect(h.onDecide).toHaveBeenNthCalledWith(1, "a1", true);
    expect(h.onDecide).toHaveBeenNthCalledWith(2, "a2", false);
    expect(h.onRejectAndDisable).toHaveBeenCalledWith(expect.objectContaining({ id: "a2", system_id: "s2" }));
    expect(screen.getAllByText(t.sys_approve).length).toBe(2);
  });

  it("renders nothing when nothing is waiting", () => {
    mount([]);
    expect(screen.queryByTestId("approval-card")).toBeNull();
  });
});
