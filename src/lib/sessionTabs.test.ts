import { describe, expect, it, beforeEach } from "vitest";
import { restoreSessionTabs, saveSessionTabs, SESSION_TABS_KEY } from "./sessionTabs";
import type { Tab } from "../components/TabBar";

beforeEach(() => {
  localStorage.clear();
});

describe("sessionTabs", () => {
  it("存下並還原 cwd 與 AI 摘要", () => {
    const tabs: Tab[] = [
      { id: "t1", title: "Tab 1", type: "terminal", cwd: "/repo/aiterm", aiSummary: "跑了建置" },
    ];
    saveSessionTabs(tabs);
    const restored = restoreSessionTabs()!;
    expect(restored[0].cwd).toBe("/repo/aiterm");
    expect(restored[0].aiSummary).toBe("跑了建置");
  });

  // 舊版存的資料沒有這兩個欄位。缺欄位不可以讓整份還原失敗——那會讓使用者
  // 的分頁全部消失。
  it("讀得懂沒有新欄位的舊格式", () => {
    localStorage.setItem(
      SESSION_TABS_KEY,
      JSON.stringify([{ title: "Old", type: "terminal" }]),
    );
    const restored = restoreSessionTabs()!;
    expect(restored).toHaveLength(1);
    expect(restored[0].title).toBe("Old");
    expect(restored[0].cwd).toBeUndefined();
  });

  it("每次還原都給新的 id", () => {
    saveSessionTabs([{ id: "old-id", title: "Tab 1", type: "terminal" }]);
    expect(restoreSessionTabs()![0].id).not.toBe("old-id");
  });

  it("內容壞掉時回 null 而不是丟例外", () => {
    localStorage.setItem(SESSION_TABS_KEY, "{ not json");
    expect(restoreSessionTabs()).toBeNull();
  });

  it("空陣列回 null（沒有可還原的東西）", () => {
    localStorage.setItem(SESSION_TABS_KEY, "[]");
    expect(restoreSessionTabs()).toBeNull();
  });

  // 只存需要的欄位：ptySessionId、attention、agentProgress 這些重開後就沒意義了，
  // 存下來只會讓還原出來的分頁帶著假狀態。
  it("不存執行期才有意義的欄位", () => {
    saveSessionTabs([{
      id: "t1", title: "Tab 1", type: "terminal",
      ptySessionId: "pty-1", attention: "done", agentProgress: { done: 1, total: 2 },
    }]);
    const raw = JSON.parse(localStorage.getItem(SESSION_TABS_KEY)!);
    expect(raw[0]).not.toHaveProperty("ptySessionId");
    expect(raw[0]).not.toHaveProperty("attention");
    expect(raw[0]).not.toHaveProperty("agentProgress");
  });
});
