import { describe, expect, it } from "vitest";
import { highlightLines, languageFor, splitHtmlLines } from "./highlight";

describe("languageFor", () => {
  it("依副檔名挑語言，不分大小寫", () => {
    expect(languageFor("a/b/Foo.java")).toBe("java");
    expect(languageFor("x.TSX")).toBe("typescript");
    expect(languageFor("run.py")).toBe("python");
    expect(languageFor("lib.rs")).toBe("rust");
    expect(languageFor("pom.xml")).toBe("xml");
  });
  it("不認得的副檔名或沒有副檔名回 null", () => {
    expect(languageFor("README")).toBeNull();
    expect(languageFor("a.unknownext")).toBeNull();
  });
});

describe("splitHtmlLines", () => {
  it("跨行的 span 在換行處關閉、下一行重開，每行都是自成一體的 HTML", () => {
    expect(splitHtmlLines('a<span class="c">x\ny</span>b')).toEqual(['a<span class="c">x</span>', '<span class="c">y</span>b']);
  });
  it("巢狀 span 跨行時依序重開", () => {
    expect(splitHtmlLines('<span class="a"><span class="b">1\n2</span></span>')).toEqual([
      '<span class="a"><span class="b">1</span></span>',
      '<span class="a"><span class="b">2</span></span>',
    ]);
  });
  it("沒有換行就原樣一行", () => {
    expect(splitHtmlLines("plain")).toEqual(["plain"]);
  });
});

describe("highlightLines", () => {
  it("Java 的關鍵字與字串會被加上 class，且行數與原文一致", () => {
    const src = 'public class A {\n  String s = "hi";\n}';
    const out = highlightLines(src, "A.java")!;
    expect(out).toHaveLength(3);
    expect(out[0]).toContain('<span class="hljs-keyword">public</span>');
    expect(out[1]).toContain('<span class="hljs-string">&quot;hi&quot;</span>');
  });
  it("多行註解的每一行都帶註解色", () => {
    const out = highlightLines("/* a\n b */\nint x;", "x.c")!;
    expect(out[0]).toContain("hljs-comment");
    expect(out[1]).toContain("hljs-comment");
    expect(out[2]).not.toContain("hljs-comment");
  });
  it("內容會被跳脫，不會變成可執行的 HTML", () => {
    const out = highlightLines('<img src=x onerror="alert(1)">', "a.java")!;
    expect(out.join("")).not.toContain("<img");
  });
  it("不認得的檔案回 null，讓呼叫端用純文字", () => {
    expect(highlightLines("hello", "NOTES")).toBeNull();
  });
});
