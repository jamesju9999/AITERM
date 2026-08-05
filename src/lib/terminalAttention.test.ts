import { describe, expect, it } from "vitest";
import {
  attentionForExitCode,
  isPastNotifyCooldown,
  notifyBodyKeyFor,
  NOTIFY_COOLDOWN_MS,
  routeAttention,
} from "./terminalAttention";

describe("routeAttention — 提示點", () => {
  it("非 active 分頁會設出對應的提示點", () => {
    expect(routeAttention({ isActiveTab: false, windowFocused: true, kind: "done" }).badge).toBe("done");
    expect(routeAttention({ isActiveTab: false, windowFocused: true, kind: "failed" }).badge).toBe("failed");
    expect(routeAttention({ isActiveTab: false, windowFocused: true, kind: "waiting" }).badge).toBe("waiting");
  });

  it("active 分頁不設提示點——使用者切回來就直接看到終端機內容了", () => {
    expect(routeAttention({ isActiveTab: true, windowFocused: true, kind: "waiting" }).badge).toBeNull();
    expect(routeAttention({ isActiveTab: true, windowFocused: false, kind: "failed" }).badge).toBeNull();
  });
});

describe("routeAttention — 桌面通知", () => {
  it("視窗有 focus 時一律不發通知——側邊欄的點就夠了", () => {
    expect(routeAttention({ isActiveTab: false, windowFocused: true, kind: "waiting" }).notify).toBe(false);
    expect(routeAttention({ isActiveTab: false, windowFocused: true, kind: "failed" }).notify).toBe(false);
    // active 分頁 + 有 focus：使用者正看著它，最不該打擾的情況。
    expect(routeAttention({ isActiveTab: true, windowFocused: true, kind: "waiting" }).notify).toBe(false);
  });

  it("視窗失焦時，waiting 與 failed 會發通知", () => {
    expect(routeAttention({ isActiveTab: false, windowFocused: false, kind: "waiting" }).notify).toBe(true);
    expect(routeAttention({ isActiveTab: false, windowFocused: false, kind: "failed" }).notify).toBe(true);
  });

  it("done 永遠不發通知——指令單純跑完不緊急", () => {
    expect(routeAttention({ isActiveTab: false, windowFocused: false, kind: "done" }).notify).toBe(false);
  });

  // 設計文件明確標記為必測：這是實作時最可能被錯誤合併掉的一條規則。
  // 使用者人不在 app 前面時，「它是 active 分頁」不代表有人在看。
  it("失焦 + active 分頁 + waiting → 不設提示點，但要發通知", () => {
    const r = routeAttention({ isActiveTab: true, windowFocused: false, kind: "waiting" });
    expect(r.badge).toBeNull();
    expect(r.notify).toBe(true);
  });
});

describe("attentionForExitCode", () => {
  it("exit code 0 → done", () => {
    expect(attentionForExitCode(0)).toBe("done");
  });

  it("非 0 的 exit code → failed", () => {
    expect(attentionForExitCode(1)).toBe("failed");
    expect(attentionForExitCode(-1)).toBe("failed");
    expect(attentionForExitCode(127)).toBe("failed");
  });
});

describe("notifyBodyKeyFor", () => {
  // 曾經被 reviewer 對調過一次，全部 563 個測試照樣綠燈——這三條就是專門
  // 釘住這個對應關係，避免同樣的錯誤再發生一次卻沒有測試會發現。
  it("waiting → terminal_notify_waiting", () => {
    expect(notifyBodyKeyFor("waiting")).toBe("terminal_notify_waiting");
  });

  it("failed → terminal_notify_failed", () => {
    expect(notifyBodyKeyFor("failed")).toBe("terminal_notify_failed");
  });

  it("done → null——done 不發通知，沒有對應的文案", () => {
    expect(notifyBodyKeyFor("done")).toBeNull();
  });
});

describe("isPastNotifyCooldown", () => {
  it("這個分頁從來沒發過通知——直接放行", () => {
    expect(isPastNotifyCooldown(undefined, 1_000)).toBe(true);
  });

  it("在冷卻時間之內——擋下來", () => {
    const lastNotifiedAt = 1_000;
    const now = lastNotifiedAt + NOTIFY_COOLDOWN_MS - 1;
    expect(isPastNotifyCooldown(lastNotifiedAt, now)).toBe(false);
  });

  it("剛好在冷卻時間的邊界上——放行", () => {
    const lastNotifiedAt = 1_000;
    const now = lastNotifiedAt + NOTIFY_COOLDOWN_MS;
    expect(isPastNotifyCooldown(lastNotifiedAt, now)).toBe(true);
  });

  it("超過冷卻時間——放行", () => {
    const lastNotifiedAt = 1_000;
    const now = lastNotifiedAt + NOTIFY_COOLDOWN_MS + 5_000;
    expect(isPastNotifyCooldown(lastNotifiedAt, now)).toBe(true);
  });
});
