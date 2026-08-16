import { describe, expect, it, beforeEach } from "vitest";
import { recordProject, listRecentProjects, RECENT_PROJECTS_KEY, MAX_RECENT_PROJECTS } from "./recentProjects";

beforeEach(() => {
  localStorage.clear();
});

describe("recentProjects", () => {
  it("記錄下來的目錄讀得回來", () => {
    recordProject("/repo/aiterm");
    expect(listRecentProjects().map((p) => p.path)).toEqual(["/repo/aiterm"]);
  });

  it("最近使用的排在最前面", () => {
    recordProject("/a");
    recordProject("/b");
    expect(listRecentProjects().map((p) => p.path)).toEqual(["/b", "/a"]);
  });

  // 同一個目錄反覆進出很常見，不去重的話清單會被同一筆塞滿。
  it("同一個目錄只留一筆，並移到最前面", () => {
    recordProject("/a");
    recordProject("/b");
    recordProject("/a");
    expect(listRecentProjects().map((p) => p.path)).toEqual(["/a", "/b"]);
  });

  it(`最多保留 ${MAX_RECENT_PROJECTS} 筆，超過就丟掉最舊的`, () => {
    for (let i = 0; i < MAX_RECENT_PROJECTS + 5; i++) recordProject(`/p${i}`);
    const list = listRecentProjects();
    expect(list).toHaveLength(MAX_RECENT_PROJECTS);
    expect(list[0].path).toBe(`/p${MAX_RECENT_PROJECTS + 4}`);
    expect(list.map((p) => p.path)).not.toContain("/p0");
  });

  it("內容壞掉時回空陣列而不是丟例外", () => {
    localStorage.setItem(RECENT_PROJECTS_KEY, "{ not json");
    expect(listRecentProjects()).toEqual([]);
  });

  it("空字串不記錄", () => {
    recordProject("");
    expect(listRecentProjects()).toEqual([]);
  });
});
