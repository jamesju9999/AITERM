import { describe, expect, it, vi, beforeEach } from "vitest";
import { act } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

// 這個測試直接呼叫捕捉到的事件 callback（不透過 Testing Library 的包裝），
// 所以要自己開 act 環境旗標。**只設在這個檔案**：設成全域會讓 React 對
// 所有測試檢查 act 包裝，可能在既有的一百多個測試檔裡冒出一堆警告。
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const approveMock = vi.fn();
const denyMock = vi.fn();
let pendingCb: ((p: { requestId: string; tabId: string; displayName: string }) => void) | null = null;

vi.mock("../../ipc/share", () => ({
  shareApprove: (...a: unknown[]) => approveMock(...a),
  shareDeny: (...a: unknown[]) => denyMock(...a),
  onSharePendingRequest: (cb: (p: never) => void) => {
    pendingCb = cb as never;
    return Promise.resolve(() => {});
  },
}));

vi.mock("../../contexts/LocaleContext", async () => {
  const { translations } = await import("../../lib/i18n");
  return { useLocale: () => ({ t: translations["zh-TW"], locale: "zh-TW", setLocale: () => {} }) };
});

import { ConsentDialog } from "./index";

// `id`（React 分頁 id）跟 `ptySessionId`（PTY session id）故意用不同字串
// ——真實情況這兩個是不同的 UUID 命名空間。若測試不小心兩者用同一個值
// （之前這裡就是這樣寫的），比對邏輯就算寫錯（比對到 `id` 而不是
// `ptySessionId`）也會巧合地測過，抓不到「畫面上顯示一串醜 UUID」這個
// 實機測試才發現的 bug。
const TABS = [{ id: "react-tab-1", title: "Claude Code", ptySessionId: "pty-session-1" }];

beforeEach(() => {
  approveMock.mockReset().mockResolvedValue({ kind: "approved", viewerId: "v1" });
  denyMock.mockReset().mockResolvedValue(undefined);
  pendingCb = null;
});

async function arrive() {
  await vi.waitFor(() => expect(pendingCb).toBeTruthy());
  await act(async () => {
    pendingCb!({ requestId: "r1", tabId: "pty-session-1", displayName: "Alice" });
  });
}

describe("ConsentDialog", () => {
  it("stays out of the way until a request arrives", () => {
    render(<ConsentDialog tabs={TABS} />);
    expect(screen.queryByText(/想連進/)).not.toBeInTheDocument();
  });

  it("names who is asking and which tab", async () => {
    render(<ConsentDialog tabs={TABS} />);
    await arrive();
    expect(screen.getByText(/Alice/)).toBeInTheDocument();
    expect(screen.getByText(/Claude Code/)).toBeInTheDocument();
  });

  it("never shows a verification code of its own", async () => {
    // **這是整個設計的關鍵。** 主控端看得到自己的碼就會照抄而不問對方，
    // 人工核對變成自欺。後端根本不送那個值過來（PendingRequest 型別上就
    // 沒有），這個測試守著「不要哪天為了『方便』把它加回來」。
    render(<ConsentDialog tabs={TABS} />);
    await arrive();
    // 畫面上不該有任何 4 位數字——那會是「答案」，使用者會照抄而不問對方。
    expect(document.body.textContent).not.toMatch(/\b\d{4}\b(?!\d)/);
  });

  it("sends the typed code to the backend rather than comparing here", async () => {
    render(<ConsentDialog tabs={TABS} />);
    await arrive();

    await userEvent.type(screen.getByRole("textbox"), "4917");
    await userEvent.click(screen.getByRole("button", { name: /可以控制/ }));

    expect(approveMock).toHaveBeenCalledWith("r1", "control", "4917");
  });

  it("tells the host when the code did not match, and closes", async () => {
    // 輸錯直接拒絕，不給重試——攻擊者只有 1/10000 的一發機會，給重試等於
    // 送他一萬次。
    approveMock.mockResolvedValue({ kind: "codeMismatch" });
    render(<ConsentDialog tabs={TABS} />);
    await arrive();

    await userEvent.type(screen.getByRole("textbox"), "1234");
    await userEvent.click(screen.getByRole("button", { name: /只能看/ }));

    expect(await screen.findByText(/驗證碼不符/)).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  it("explains when control is already taken and keeps the dialog open", async () => {
    // 這種情況請求還在（後端會放回待審），主控端可以改用唯讀重新裁決。
    approveMock.mockResolvedValue({ kind: "controlTaken" });
    render(<ConsentDialog tabs={TABS} />);
    await arrive();

    await userEvent.type(screen.getByRole("textbox"), "4917");
    await userEvent.click(screen.getByRole("button", { name: /可以控制/ }));

    expect(await screen.findByText(/控制權已經給了別人/)).toBeInTheDocument();
    expect(screen.getByRole("textbox")).toBeInTheDocument();
  });

  it("denies without needing a code", async () => {
    render(<ConsentDialog tabs={TABS} />);
    await arrive();
    await userEvent.click(screen.getByRole("button", { name: /拒絕/ }));
    expect(denyMock).toHaveBeenCalledWith("r1");
  });
});
