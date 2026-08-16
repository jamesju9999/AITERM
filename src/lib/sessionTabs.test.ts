import { describe, expect, it, beforeEach } from "vitest";
import { restoreSessionTabs, saveSessionTabs, SESSION_TABS_KEY } from "./sessionTabs";
import type { Tab } from "../components/TabBar";

beforeEach(() => {
  localStorage.clear();
});

describe("sessionTabs", () => {
  // 少了這個，還原的分頁都會開在全域 aiterm_last_cwd，接著第一次 cwd 輪詢
  // 就把每個分頁的 cwd 覆蓋成同一個值——存下來的目錄活不過開機後兩秒。
  it("還原時把存下的目錄當成 PTY 的起始目錄", () => {
    saveSessionTabs([
      { id: "a", title: "A", type: "terminal", cwd: "/repo/foo" },
      { id: "b", title: "B", type: "terminal", cwd: "/repo/bar" },
    ]);
    const restored = restoreSessionTabs()!;
    expect(restored.map((t) => t.initialCwd)).toEqual(["/repo/foo", "/repo/bar"]);
  });

  // aiSummary 會餵進標題列，而標題列沒有「上次」的框架——這個 session 還沒
  // 跑任何指令就顯示上次的摘要等於在說謊。
  it("還原的摘要進 lastSessionSummary，不進 aiSummary", () => {
    saveSessionTabs([{ id: "t1", title: "T", type: "terminal", aiSummary: "跑了建置" }]);
    const restored = restoreSessionTabs()!;
    expect(restored[0].lastSessionSummary).toBe("跑了建置");
    expect(restored[0].aiSummary).toBeUndefined();
  });

  // 連續重開兩次而中間沒下過指令時，摘要不可以就這樣消失。
  it("這個 session 沒有新摘要時，把上次那份留著", () => {
    saveSessionTabs([{ id: "t1", title: "T", type: "terminal", lastSessionSummary: "上次做的事" }]);
    expect(restoreSessionTabs()![0].lastSessionSummary).toBe("上次做的事");
  });

  it("這個 session 有新摘要時蓋過舊的", () => {
    saveSessionTabs([{
      id: "t1", title: "T", type: "terminal",
      aiSummary: "這次做的事", lastSessionSummary: "上次做的事",
    }]);
    expect(restoreSessionTabs()![0].lastSessionSummary).toBe("這次做的事");
  });

  it("存下並還原 cwd 與 AI 摘要", () => {
    const tabs: Tab[] = [
      { id: "t1", title: "Tab 1", type: "terminal", cwd: "/repo/aiterm", aiSummary: "跑了建置" },
    ];
    saveSessionTabs(tabs);
    const restored = restoreSessionTabs()!;
    expect(restored[0].cwd).toBe("/repo/aiterm");
    // 摘要還原到 lastSessionSummary，理由見上面那條測試。
    expect(restored[0].lastSessionSummary).toBe("跑了建置");
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
