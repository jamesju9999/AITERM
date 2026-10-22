import { beforeEach, describe, expect, it, vi } from "vitest";

import { translations } from "../../lib/i18n";

const setFocus = vi.fn().mockResolvedValue(undefined);
vi.mock("@tauri-apps/api/webview", () => ({ getCurrentWebview: () => ({ setFocus }) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ confirm: vi.fn() }));
vi.mock("../../ipc/agents", () => ({ initRepo: vi.fn().mockResolvedValue(undefined) }));

import { confirm } from "@tauri-apps/plugin-dialog";
import { initRepo } from "../../ipc/agents";
import { confirmInitRepo } from "./repoInit";

const t = translations.en;

beforeEach(() => {
  vi.clearAllMocks();
});

describe("confirmInitRepo", () => {
  // 實機（macOS）：關掉原生確認視窗後文字框有外框與游標，卻打不了字——網頁檢視沒拿回鍵盤焦點。
  it("關掉確認視窗後把焦點還給網頁檢視：按確定或取消都要", async () => {
    vi.mocked(confirm).mockResolvedValueOnce(true);
    expect(await confirmInitRepo(t, "/r", "not_repo", "p")).toBeNull();
    expect(setFocus).toHaveBeenCalledTimes(1);
    expect(initRepo).toHaveBeenCalledWith("/r", "p");

    vi.mocked(confirm).mockResolvedValueOnce(false);
    expect(await confirmInitRepo(t, "/r", "not_repo", "p")).toBeNull();
    expect(setFocus).toHaveBeenCalledTimes(2);
    expect(initRepo).toHaveBeenCalledTimes(1);
  });

  it("還焦點失敗（不在 Tauri 裡、沒權限）不影響流程", async () => {
    setFocus.mockRejectedValueOnce(new Error("denied"));
    vi.mocked(confirm).mockResolvedValueOnce(true);
    expect(await confirmInitRepo(t, "/r", "not_repo", "p")).toBeNull();
    expect(initRepo).toHaveBeenCalled();
  });
});
