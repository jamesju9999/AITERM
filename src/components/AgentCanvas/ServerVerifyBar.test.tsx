import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { LocaleProvider } from "../../contexts/LocaleContext";

const role = vi.hoisted(() => ({ value: "admin" as "admin" | "member" | null }));
vi.mock("../../hooks/useIntakeRemote", () => ({ useIntakeRemote: () => ({ connected: true, online: true, addr: "x", name: "我", role: role.value, error: null }) }));
vi.mock("../../ipc/serverVerify", () => ({
  getVerify: vi.fn(),
  setVerify: vi.fn(),
  getProjectDeny: vi.fn(),
  getGlobalDeny: vi.fn(),
  setProjectDeny: vi.fn(),
  setGlobalDeny: vi.fn(),
  resetGlobalDeny: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn(() => Promise.resolve(true)) }));

import { getProjectDeny, getVerify, setVerify } from "../../ipc/serverVerify";
import { ServerVerifyBar } from "./ServerVerifyBar";

const mount = (props: Partial<Parameters<typeof ServerVerifyBar>[0]> = {}) => {
  const onError = vi.fn();
  const utils = render(
    <LocaleProvider>
      <ServerVerifyBar projectId="remote:p1" onError={onError} {...props} />
    </LocaleProvider>,
  );
  return { onError, ...utils };
};

beforeEach(() => {
  role.value = "admin";
  vi.mocked(getVerify).mockResolvedValue({ enabled: false, has_verifier: true });
  vi.mocked(setVerify).mockResolvedValue(undefined);
  vi.mocked(getProjectDeny).mockResolvedValue({ global: [], overrides: [], effective: [] });
});
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ServerVerifyBar", () => {
  it("本機專案沒有 server 驗收：什麼都不畫", async () => {
    vi.mocked(getVerify).mockResolvedValue(null);
    const { container } = mount({ projectId: "local-uuid" });
    await waitFor(() => expect(getVerify).toHaveBeenCalledWith("local-uuid"));
    expect(container).toBeEmptyDOMElement();
  });

  it("管理員：顯示狀態，按一下切換並送給 server；也看得到禁止清單入口", async () => {
    mount();
    const btn = await screen.findByTestId("ac-server-verify");
    expect(btn).toHaveTextContent("由 server 驗收 · 未開啟");
    fireEvent.click(btn);
    await waitFor(() => expect(btn).toHaveTextContent("已開啟"));
    expect(setVerify).toHaveBeenCalledWith("remote:p1", true);
    expect(screen.getByTestId("ac-deny-open")).toBeInTheDocument();
  });

  it("沒有驗收者：開關停用並說明要先加驗收者", async () => {
    vi.mocked(getVerify).mockResolvedValue({ enabled: false, has_verifier: false });
    mount();
    const btn = await screen.findByTestId("ac-server-verify");
    expect(btn).toBeDisabled();
    expect(btn.getAttribute("title")).toContain("驗收者");
  });

  it("成員：看得到狀態但不能開關，也沒有禁止清單入口", async () => {
    role.value = "member";
    vi.mocked(getVerify).mockResolvedValue({ enabled: true, has_verifier: true });
    mount();
    const btn = await screen.findByTestId("ac-server-verify");
    expect(btn).toBeDisabled();
    expect(btn).toHaveTextContent("已開啟");
    expect(screen.queryByTestId("ac-deny-open")).toBeNull();
  });

  it("server 拒絕時把原因交給 onError，狀態不變", async () => {
    vi.mocked(setVerify).mockRejectedValue("這個專案還沒設定工作資料夾");
    const { onError } = mount();
    const btn = await screen.findByTestId("ac-server-verify");
    fireEvent.click(btn);
    await waitFor(() => expect(onError).toHaveBeenCalledWith("這個專案還沒設定工作資料夾"));
    expect(btn).toHaveTextContent("未開啟");
  });
});
