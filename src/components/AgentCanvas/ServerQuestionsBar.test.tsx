import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

const role = vi.hoisted(() => ({ value: "admin" as "admin" | "member" }));
vi.mock("../../hooks/useIntakeRemote", () => ({ useIntakeRemote: () => ({ connected: true, online: true, addr: "x", name: "我", role: role.value, error: null }) }));
vi.mock("../../ipc/serverQuestions", async (orig) => ({ ...(await orig<typeof import("../../ipc/serverQuestions")>()), listServerQuestions: vi.fn(), answerServerQuestion: vi.fn(), cancelServerQuestion: vi.fn() }));

import { answerServerQuestion, cancelServerQuestion, listServerQuestions, type ServerQuestion } from "../../ipc/serverQuestions";
import { ServerQuestionsBar } from "./ServerQuestionsBar";

const q = (id: number, over: Partial<ServerQuestion> = {}): ServerQuestion => ({ id, project_id: "p1", card_id: "c1", agent_id: "a1", question: "用 A 還是 B？", options: JSON.stringify(["A", "B"]), recommended: "A", context: "兩個都能用", status: "open", answer: null, answered_by: null, answered_via: null, created_at: 1, answered_at: null, ...over });

const mount = (active = true) =>
  render(
    <LocaleProvider>
      <ServerQuestionsBar projectId="remote:p1" agentName={(id) => (id === "a1" ? "工程師1" : "?")} active={active} />
    </LocaleProvider>,
  );

beforeEach(() => {
  role.value = "admin";
  vi.mocked(listServerQuestions).mockResolvedValue({ open: [q(7)], closed: [] });
  vi.mocked(answerServerQuestion).mockResolvedValue(undefined);
  vi.mocked(cancelServerQuestion).mockResolvedValue(undefined);
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ServerQuestionsBar", () => {
  it("沒有待回答的問題時什麼都不畫；不在使用中的畫布不去問 server", async () => {
    vi.mocked(listServerQuestions).mockResolvedValue({ open: [], closed: [] });
    const { container } = mount();
    await waitFor(() => expect(listServerQuestions).toHaveBeenCalledWith("remote:p1"));
    expect(container).toBeEmptyDOMElement();
    cleanup();
    vi.mocked(listServerQuestions).mockClear();
    mount(false);
    await act(async () => {});
    expect(listServerQuestions).not.toHaveBeenCalled();
  });

  it("按下去看到問題、背景與建議；點選項就回答", async () => {
    mount();
    fireEvent.click(await screen.findByTestId("ac-questions"));
    expect(screen.getByText("用 A 還是 B？")).toBeInTheDocument();
    expect(screen.getByText(/兩個都能用/)).toBeInTheDocument();
    expect(screen.getByText(/Agent 建議：A/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "B" }));
    await waitFor(() => expect(answerServerQuestion).toHaveBeenCalledWith(7, "B"));
  });

  it("沒有選項時自己打答案；空白不能送；Enter 也能送", async () => {
    vi.mocked(listServerQuestions).mockResolvedValue({ open: [q(8, { options: "[]", recommended: null, context: null })], closed: [] });
    mount();
    fireEvent.click(await screen.findByTestId("ac-questions"));
    const send = screen.getByRole("button", { name: "送出" });
    expect(send).toBeDisabled();
    const input = screen.getByPlaceholderText("自己輸入答案");
    fireEvent.change(input, { target: { value: "  小花  " } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(answerServerQuestion).toHaveBeenCalledWith(8, "小花"));
  });

  it("只有管理員看得到取消；成員可以回答", async () => {
    role.value = "member";
    mount();
    fireEvent.click(await screen.findByTestId("ac-questions"));
    expect(screen.queryByRole("button", { name: "取消這個問題" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "A" }));
    await waitFor(() => expect(answerServerQuestion).toHaveBeenCalledWith(7, "A"));
    cleanup();
    role.value = "admin";
    mount();
    fireEvent.click(await screen.findByTestId("ac-questions"));
    fireEvent.click(screen.getByRole("button", { name: "取消這個問題" }));
    await waitFor(() => expect(cancelServerQuestion).toHaveBeenCalledWith(7));
  });

  it("問題都答完後對話框會收起來，之後來新問題不會自己彈出", async () => {
    mount();
    fireEvent.click(await screen.findByTestId("ac-questions"));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    vi.mocked(listServerQuestions).mockResolvedValue({ open: [], closed: [] });
    fireEvent.click(screen.getByRole("button", { name: "A" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    vi.mocked(listServerQuestions).mockResolvedValue({ open: [q(9)], closed: [] });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 3100));
    });
    expect(await screen.findByTestId("ac-questions")).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("回答失敗（例如已被別人答了）顯示原因", async () => {
    vi.mocked(answerServerQuestion).mockRejectedValue("這個問題已經有人回答了");
    mount();
    fireEvent.click(await screen.findByTestId("ac-questions"));
    fireEvent.click(screen.getByRole("button", { name: "A" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("已經有人回答");
  });
});
