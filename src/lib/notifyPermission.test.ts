import { describe, it, expect, vi, beforeEach } from "vitest";
import { ensureNotificationPermission, resetNotificationPermissionForTests } from "./notifyPermission";

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn(),
  requestPermission: vi.fn(),
}));

import { isPermissionGranted, requestPermission } from "@tauri-apps/plugin-notification";

describe("notifyPermission", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetNotificationPermissionForTests();
  });

  it("does not cache a transient failure as a denial", async () => {
    vi.mocked(isPermissionGranted).mockRejectedValueOnce(new Error("IPC failed"));
    await expect(ensureNotificationPermission()).resolves.toBe(false);

    vi.mocked(isPermissionGranted).mockResolvedValueOnce(true);
    await expect(ensureNotificationPermission()).resolves.toBe(true);
    expect(isPermissionGranted).toHaveBeenCalledTimes(2);
  });

  it("shares a single in-flight request across concurrent callers", async () => {
    vi.mocked(isPermissionGranted).mockResolvedValue(true);

    const first = ensureNotificationPermission();
    const second = ensureNotificationPermission();
    const [a, b] = await Promise.all([first, second]);

    expect(isPermissionGranted).toHaveBeenCalledTimes(1);
    expect(a).toBe(true);
    expect(b).toBe(true);
  });

  it("caches a genuine denial", async () => {
    vi.mocked(isPermissionGranted).mockResolvedValue(false);
    vi.mocked(requestPermission).mockResolvedValue("denied");

    await expect(ensureNotificationPermission()).resolves.toBe(false);
    await expect(ensureNotificationPermission()).resolves.toBe(false);

    expect(requestPermission).toHaveBeenCalledTimes(1);
  });
});
