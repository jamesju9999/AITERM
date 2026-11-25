import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

vi.mock("../../ipc/tasks", () => ({ approveMerge: vi.fn() }));

import { approveMerge } from "../../ipc/tasks";
import { ServerMergeBanner } from "./ServerMergeBanner";

const mount = (cards: Array<{ id: string; title: string }>) => {
  const onError = vi.fn();
  const onNotice = vi.fn();
  const utils = render(
    <LocaleProvider>
      <ServerMergeBanner mergeCards={cards} onError={onError} onNotice={onNotice} />
    </LocaleProvider>,
  );
  return { onError, onNotice, ...utils };
};

beforeEach(() => {
  vi.mocked(approveMerge).mockResolvedValue("已合併：工程師1");
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ServerMergeBanner", () => {
  it("沒有待核准的合併卡：什麼都不畫", () => {
    const { container } = mount([]);
    expect(container).toBeEmptyDOMElement();
  });

  it("有待核准的合併卡：顯示說明與按鈕", () => {
    mount([{ id: "m1", title: "合併待核准" }]);
    expect(screen.getByTestId("ac-merge-banner")).toHaveTextContent("全部模組都審查通過了");
    expect(screen.getByTestId("ac-merge-approve")).toHaveTextContent("核准合併");
  });

  it("核准合併要按兩下：第一下只問確認，第二下才送；成功後把 server 的說明顯示出來", async () => {
    const { onNotice } = mount([{ id: "m1", title: "合併待核准" }]);
    const btn = screen.getByTestId("ac-merge-approve");
    expect(btn).toHaveTextContent("核准合併");
    fireEvent.click(btn);
    expect(btn).toHaveTextContent("確定合併？");
    expect(approveMerge).not.toHaveBeenCalled();
    fireEvent.click(btn);
    await waitFor(() => expect(approveMerge).toHaveBeenCalledWith("m1"));
    await waitFor(() => expect(onNotice).toHaveBeenCalledWith("server 回報：已合併：工程師1"));
  });

  it("離開按鈕（失焦）就取消確認，不會留著半個確認狀態", async () => {
    mount([{ id: "m1", title: "合併待核准" }]);
    const btn = screen.getByTestId("ac-merge-approve");
    fireEvent.click(btn);
    expect(btn).toHaveTextContent("確定合併？");
    fireEvent.blur(btn);
    expect(btn).toHaveTextContent("核准合併");
  });

  it("合併被拒絕（衝突、不是管理員）：錯誤交出去，不當成成功", async () => {
    vi.mocked(approveMerge).mockRejectedValue("合併「工程師1」失敗（已還原這一個）");
    const { onError, onNotice } = mount([{ id: "m1", title: "合併待核准" }]);
    const btn = screen.getByTestId("ac-merge-approve");
    fireEvent.click(btn);
    fireEvent.click(btn);
    await waitFor(() => expect(onError).toHaveBeenCalledWith("合併「工程師1」失敗（已還原這一個）"));
    expect(onNotice).not.toHaveBeenCalled();
  });
});
