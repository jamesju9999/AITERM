import { describe, it, expect, vi } from "vitest";
import { runCloseGuard } from "./closeTabGuard";

describe("runCloseGuard", () => {
  it("沒有 guard 時直接放行，且不切換分頁", async () => {
    const setActiveId = vi.fn();
    await expect(
      runCloseGuard("tab-2", "tab-1", undefined, setActiveId)
    ).resolves.toBe(true);
    expect(setActiveId).not.toHaveBeenCalled();
  });

  it("有 guard 但目標不是當前分頁：先切過去再問", async () => {
    const setActiveId = vi.fn();
    const order: string[] = [];
    const guard = vi.fn(async () => { order.push("guard"); return true; });
    setActiveId.mockImplementation(() => { order.push("switch"); });

    await runCloseGuard("tab-2", "tab-1", guard, setActiveId);

    expect(setActiveId).toHaveBeenCalledWith("tab-2");
    expect(order).toEqual(["switch", "guard"]);
  });

  it("有 guard 且目標已是當前分頁：不重複切換", async () => {
    const setActiveId = vi.fn();
    const guard = vi.fn(async () => true);

    await runCloseGuard("tab-1", "tab-1", guard, setActiveId);

    expect(setActiveId).not.toHaveBeenCalled();
    expect(guard).toHaveBeenCalledTimes(1);
  });

  it("guard 回傳 false 時原樣傳回", async () => {
    const guard = vi.fn(async () => false);
    await expect(
      runCloseGuard("tab-1", "tab-1", guard, vi.fn())
    ).resolves.toBe(false);
  });

  // 釘住一個已知的取捨（不是理想行為，但先讓它不會被無聲改掉）：
  // 在背景分頁按 ✕ 之後選「取消」，焦點不會切回原本那個分頁，使用者會
  // 停在自己沒主動選擇的分頁上。要問確認框就一定得先讓那個分頁可見，
  // 而目前沒有還原焦點的機制。若哪天決定要還原，這個測試會紅，屆時
  // 應該更新它，而不是刪掉。
  it("背景分頁被取消關閉時，焦點仍留在該分頁（已知取捨）", async () => {
    const setActiveId = vi.fn();
    const guard = vi.fn(async () => false);

    await expect(
      runCloseGuard("tab-2", "tab-1", guard, setActiveId)
    ).resolves.toBe(false);

    expect(setActiveId).toHaveBeenCalledTimes(1);
    expect(setActiveId).toHaveBeenCalledWith("tab-2");
    expect(setActiveId).not.toHaveBeenCalledWith("tab-1");
  });
});
