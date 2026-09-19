import { describe, it, expect, beforeEach } from "vitest";
import { saveSessionTabs, restoreSessionTabs } from "./sessionTabs";
import type { Tab } from "../components/TabBar";

beforeEach(() => localStorage.clear());

describe("session tab persistence", () => {
  it("never persists initialCommand, so a restart cannot re-run a launch command", () => {
    const tab: Tab = {
      id: "t1",
      title: "Terminal",
      type: "terminal",
      cwd: "/proj",
      initialCommand: "rm -rf build",
    };
    saveSessionTabs([tab]);
    // 不依賴 storage key 的名字：掃過 localStorage 裡所有的值。
    // 用 length/key(i) 而不是 Object.keys(localStorage)：test-setup.ts 的
    // MemoryStorage 把資料放在私有 Map，Object.keys 永遠回傳空陣列。
    const everythingStored = Array.from({ length: localStorage.length }, (_, i) =>
      localStorage.getItem(localStorage.key(i)!),
    ).join("\n");
    expect(everythingStored).toContain("/proj"); // 確認真的有存東西，否則下一行是空斷言
    expect(everythingStored).not.toContain("rm -rf build");
    expect(restoreSessionTabs()?.[0].initialCommand).toBeUndefined();
  });
});
