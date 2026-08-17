import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

const usageSummaryMock = vi.fn();
vi.mock("../../ipc/usage", () => ({
  usageSummary: (...args: unknown[]) => usageSummaryMock(...args),
}));

import { LocaleProvider } from "../../contexts/LocaleContext";
import { UsageSection } from "./UsageSection";

function renderUsage() {
  render(
    <LocaleProvider>
      <UsageSection />
    </LocaleProvider>,
  );
}

const entry = {
  provider_id: "anthropic", model: "claude-opus-5", requests: 12,
  prompt_tokens: 1000, completion_tokens: 500,
  cache_read_tokens: 0, cache_write_tokens: 0, estimated_cost_usd: 0.42,
};

beforeEach(() => {
  usageSummaryMock.mockReset();
});

describe("UsageSection", () => {
  it("顯示今日各模型的請求數與 token 數", async () => {
    usageSummaryMock.mockResolvedValue([entry]);
    renderUsage();
    expect(await screen.findByText("claude-opus-5")).toBeInTheDocument();
    expect(screen.getByText("12")).toBeInTheDocument();
    // prompt + completion = 1500。這裡寫死千分位逗號：實作用 toLocaleString()，
    // 而它跟著執行環境的 locale 走。zh-TW 與 en-US（Node 在此的預設）都是
    // "1,500"，本專案只有這兩種語言，所以斷言成立；但系統 locale 為 de-DE 的
    // 機器會拿到 "1.500" 而讓這條失敗——失敗訊息看不出這個原因，故記在這裡。
    expect(screen.getByText("1,500")).toBeInTheDocument();
  });

  // 抓不到用量不能讓整個首頁掛掉，也不能顯示成 0——那是謊話。
  it("查詢失敗時顯示無法取得，而不是 0", async () => {
    usageSummaryMock.mockRejectedValue(new Error("boom"));
    renderUsage();
    expect(await screen.findByText("無法取得用量資料")).toBeInTheDocument();
    expect(screen.queryByText("0")).not.toBeInTheDocument();
  });

  it("今天還沒用過時顯示空狀態", async () => {
    usageSummaryMock.mockResolvedValue([]);
    renderUsage();
    expect(await screen.findByText("今天還沒有用量")).toBeInTheDocument();
  });

  it("只查今天", async () => {
    usageSummaryMock.mockResolvedValue([]);
    renderUsage();
    await waitFor(() => expect(usageSummaryMock).toHaveBeenCalledWith("today"));
  });

  // 多筆才分得出「列出全部」與「只列第一筆」。
  it("多個模型全部列出", async () => {
    usageSummaryMock.mockResolvedValue([
      entry,
      { ...entry, model: "claude-sonnet-5", requests: 3 },
    ]);
    renderUsage();
    expect(await screen.findByText("claude-opus-5")).toBeInTheDocument();
    expect(screen.getByText("claude-sonnet-5")).toBeInTheDocument();
  });
});
