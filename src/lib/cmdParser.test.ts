import { describe, expect, it } from "vitest";
import { parseCmdTags, truncateAtCmdTag } from "./cmdParser";

describe("parseCmdTags", () => {
  it("returns a single text part for pure text", () => {
    const parts = parseCmdTags("just some words");
    expect(parts).toEqual([{ type: "text", content: "just some words" }]);
  });

  it("returns empty array for empty string", () => {
    expect(parseCmdTags("")).toEqual([]);
  });

  it("extracts a single single-line cmd", () => {
    const parts = parseCmdTags("試試 <cmd>ls -la</cmd> 看看");
    expect(parts).toEqual([
      { type: "text", content: "試試 " },
      { type: "cmd", content: "ls -la", multiline: false },
      { type: "text", content: " 看看" },
    ]);
  });

  it("extracts multiple cmds", () => {
    const parts = parseCmdTags("先 <cmd>cd /tmp</cmd> 再 <cmd>ls</cmd>");
    expect(parts).toHaveLength(4);
    expect(parts[1]).toEqual({ type: "cmd", content: "cd /tmp", multiline: false });
    expect(parts[3]).toEqual({ type: "cmd", content: "ls", multiline: false });
  });

  it("trims whitespace inside cmd", () => {
    const parts = parseCmdTags("<cmd>  ls   </cmd>");
    expect(parts[0]).toEqual({ type: "cmd", content: "ls", multiline: false });
  });

  it("marks multiline=true when cmd contains newlines", () => {
    const parts = parseCmdTags("<cmd>cd /tmp\nls -la</cmd>");
    expect(parts).toHaveLength(1);
    expect(parts[0]).toEqual({
      type: "cmd",
      content: "cd /tmp\nls -la",
      multiline: true,
    });
  });

  it("treats unclosed <cmd> as plain text", () => {
    const parts = parseCmdTags("oops <cmd>ls never closes");
    expect(parts).toEqual([
      { type: "text", content: "oops <cmd>ls never closes" },
    ]);
  });

  it("handles nested with non-greedy match (takes inner first pair)", () => {
    // Non-greedy regex matches the first complete pair: <cmd>a<cmd>b</cmd>
    // which yields cmd content "a<cmd>b". The trailing </cmd> becomes text.
    const parts = parseCmdTags("<cmd>a<cmd>b</cmd></cmd>");
    expect(parts[0]).toEqual({
      type: "cmd",
      content: "a<cmd>b",
      multiline: false,
    });
    expect(parts[1]).toEqual({ type: "text", content: "</cmd>" });
  });

  it("handles cmd at very start", () => {
    const parts = parseCmdTags("<cmd>ls</cmd> done");
    expect(parts[0]).toEqual({ type: "cmd", content: "ls", multiline: false });
    expect(parts[1]).toEqual({ type: "text", content: " done" });
  });

  it("handles cmd at very end", () => {
    const parts = parseCmdTags("run <cmd>ls</cmd>");
    expect(parts[0]).toEqual({ type: "text", content: "run " });
    expect(parts[1]).toEqual({ type: "cmd", content: "ls", multiline: false });
  });
});

/**
 * 串流途中的文字不能直接餵給 `parseCmdTags`：
 *   - 閉合標籤還沒到之前，整個 `<cmd>ls -la` 會被當純文字原樣露出來；
 *   - 閉合標籤一到，畫面上就會冒出一顆可以按的 ▶——但 Agent 等一下自己也會
 *     跑同一條指令，使用者按下去就變成執行兩次。
 * 所以串流氣泡只顯示第一個 `<cmd>` 之前的部分，指令留給最終訊息以正常的
 * CmdTag 卡片呈現。
 */
describe("truncateAtCmdTag", () => {
  it("沒有 cmd 標籤時原樣返回", () => {
    expect(truncateAtCmdTag("我來看看目前的檔案")).toBe("我來看看目前的檔案");
  });

  it("從第一個 <cmd> 起全部切掉（含已閉合的）", () => {
    expect(truncateAtCmdTag("先看一下：<cmd>ls -la</cmd> 然後再說")).toBe("先看一下：");
  });

  it("還沒閉合的 <cmd> 也要切掉", () => {
    expect(truncateAtCmdTag("先看一下：<cmd>ls -l")).toBe("先看一下：");
  });

  it("只剩半截開頭標籤時不能讓碎片閃出來", () => {
    // 開頭標籤是一個 delta 一個 delta 拼出來的，中間會經過 "<"、"<c"、"<cm"…
    expect(truncateAtCmdTag("先看一下：<cm")).toBe("先看一下：");
    expect(truncateAtCmdTag("先看一下：<")).toBe("先看一下：");
  });

  it("句子中間的 < 不算標籤碎片", () => {
    expect(truncateAtCmdTag("a < b 而且 b < c")).toBe("a < b 而且 b < c");
  });

  it("整段只有指令時返回空字串", () => {
    expect(truncateAtCmdTag("<cmd>ls</cmd>")).toBe("");
  });
});
