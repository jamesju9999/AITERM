import { describe, it, expect } from "vitest";
import { findSuggestion } from "./commandSuggestion";

// 歷史順序：舊 → 新（與 WarpInput 的 history 相同）
describe("findSuggestion", () => {
  const history = ["git status", "git stash", "ls -la", "git commit -m x"];

  it("回傳最新一筆符合開頭者的剩餘部分", () => {
    expect(findSuggestion(history, "git s")).toBe("tash"); // git stash 比 git status 新
  });
  it("空輸入不建議", () => expect(findSuggestion(history, "")).toBeNull());
  it("沒有符合的歷史", () => expect(findSuggestion(history, "docker")).toBeNull());
  it("與歷史完全相同時不建議（沒有剩餘部分）", () => {
    expect(findSuggestion(history, "ls -la")).toBeNull();
  });
  it("區分大小寫", () => expect(findSuggestion(history, "Git")).toBeNull());
  it("輸入含換行不建議", () => expect(findSuggestion(history, "git\nst")).toBeNull());
  it("歷史裡含換行的多行指令不會被拿來建議", () => {
    expect(findSuggestion(["echo a\necho b"], "echo")).toBeNull();
  });
  it("較舊但更長的符合者，不會蓋過較新的符合者", () => {
    expect(findSuggestion(["git commit --amend", "git commit"], "git c")).toBe("ommit");
  });
});
