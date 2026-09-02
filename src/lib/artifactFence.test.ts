import { describe, expect, it } from "vitest";
import { splitArtifactFence } from "./artifactFence";

describe("splitArtifactFence", () => {
  it("returns the text unchanged when there is no artifact fence", () => {
    expect(splitArtifactFence("just a reply")).toEqual({
      prose: "just a reply",
      artifact: null,
    });
  });

  it("splits prose before the fence from the document after it", () => {
    const text = "這是報告：\n\n```artifact-html\n<title>T</title>\n```\n";
    expect(splitArtifactFence(text)).toEqual({
      prose: "這是報告：",
      artifact: { kind: "html", content: "<title>T</title>" },
    });
  });

  it("recognises a chart fence", () => {
    const text = '```artifact-chart\n{"title":"S"}\n```';
    expect(splitArtifactFence(text).artifact).toEqual({
      kind: "chart",
      content: '{"title":"S"}',
    });
  });

  // 這是這個函式存在的理由。實機上模型寫了一份含 ``` 的長 HTML 報告，markdown
  // 的 fence 規則在文件中途就把它關掉：面板只拿到前半段（文件被截斷），後半段
  // 溢出成聊天泡泡裡的一大坨原始 HTML。交給 markdown 解析永遠會有這個問題，
  // 所以改成「開頭之後全部都算文件」。
  it("keeps going past a fence that appears inside the document", () => {
    const text = [
      "報告如下：",
      "```artifact-html",
      "<h1>A</h1>",
      "```",           // 文件內部的 fence，不可以在這裡截斷
      "<h2>B</h2>",
      "```",           // 真正的結尾
    ].join("\n");
    const { prose, artifact } = splitArtifactFence(text);
    expect(prose).toBe("報告如下：");
    expect(artifact?.content).toContain("<h1>A</h1>");
    expect(artifact?.content).toContain("<h2>B</h2>");
  });

  it("does not need a closing fence at all (mid-stream)", () => {
    const text = "產生中：\n```artifact-html\n<h1>half";
    expect(splitArtifactFence(text).artifact).toEqual({
      kind: "html",
      content: "<h1>half",
    });
  });

  it("ignores a fence that is not at the start of a line", () => {
    const text = "see `` `artifact-html` `` inline";
    expect(splitArtifactFence(text).artifact).toBeNull();
  });

  // html 與 chart 的規則刻意不同：HTML 文件裡合法地可能出現 ```（原本的 bug
  // 就是被它截斷），但 JSON 不會。對 chart 沿用「吃到訊息結尾」的話，模型只要
  // 在 fence 後多寫一句話，那句話就會被塞進 JSON 裡讓 parse 失敗。
  it("stops a chart at its closing fence so trailing prose cannot break the JSON", () => {
    const text = '```artifact-chart\n{"type":"bar"}\n```\n以上是圖表。';
    expect(splitArtifactFence(text).artifact).toEqual({
      kind: "chart",
      content: '{"type":"bar"}',
    });
  });

  it("still lets an html document run past a closing fence", () => {
    const text = "```artifact-html\n<h1>A</h1>\n```\n<h2>B</h2>";
    expect(splitArtifactFence(text).artifact?.content).toContain("<h2>B</h2>");
  });

  it("recognises the fence even when the model emits CRLF line endings", () => {
    const text = "這是報告：\r\n\r\n```artifact-html\r\n<title>T</title>\r\n```\r\n";
    expect(splitArtifactFence(text)).toEqual({
      prose: "這是報告：",
      artifact: { kind: "html", content: "<title>T</title>" },
    });
  });

  it("takes the first fence when the model emits more than one", () => {
    const text = "```artifact-html\n<h1>first</h1>\n```\n```artifact-html\n<h1>second</h1>\n```";
    const { artifact } = splitArtifactFence(text);
    expect(artifact?.content).toContain("first");
    // 第二份被吸收進同一份內容——prompt 明說一則訊息最多一個 artifact。
    expect(artifact?.content).toContain("second");
  });

  // ── 沒包 fence 的裸 HTML 文件保底 ──────────────────────────────────────────
  // 較弱的模型（實測 KB 問答 + 本地模型，尤其剛安裝後第一次）會不照協定包
  // ```artifact-html，直接把整份 HTML 當內文吐出來。前端因此只看到裸標籤、
  // 面板也不開。這批 case 釘住保底行為與它刻意的邊界。

  it("extracts a bare HTML document that opens the reply, keeping the trailing note as prose", () => {
    const text =
      "<!DOCTYPE html>\n<html><head><title>Report</title></head><body><h1>R</h1></body></html>\n\n" +
      "已為您產生一份報告文件，包含執行摘要與測試結果表格。";
    expect(splitArtifactFence(text)).toEqual({
      prose: "已為您產生一份報告文件，包含執行摘要與測試結果表格。",
      artifact: {
        kind: "html",
        content:
          "<!DOCTYPE html>\n<html><head><title>Report</title></head><body><h1>R</h1></body></html>",
      },
    });
  });

  it("extracts a bare <html> document even without a doctype", () => {
    const text = "<html><body><p>hi</p></body></html>";
    expect(splitArtifactFence(text).artifact).toEqual({
      kind: "html",
      content: "<html><body><p>hi</p></body></html>",
    });
  });

  it("treats a bare document that has not finished streaming as an in-progress artifact", () => {
    const text = "<!DOCTYPE html>\n<html><body><h1>half";
    expect(splitArtifactFence(text)).toEqual({
      prose: "",
      artifact: { kind: "html", content: "<!DOCTYPE html>\n<html><body><h1>half" },
    });
  });

  it("does NOT touch a reply that explains HTML in a code block", () => {
    const text = "這是一個基本頁面：\n\n```html\n<!DOCTYPE html>\n<html></html>\n```\n把它存成 .html 就能打開。";
    expect(splitArtifactFence(text).artifact).toBeNull();
  });

  it("does NOT extract a bare document that appears after some prose (start-only rule)", () => {
    const text = "報告如下：\n<!DOCTYPE html>\n<html><body>x</body></html>";
    expect(splitArtifactFence(text).artifact).toBeNull();
  });

  it("still prefers a real artifact fence over the bare-HTML fallback", () => {
    const text = "說明在前。\n\n```artifact-html\n<html><body>doc</body></html>\n```";
    expect(splitArtifactFence(text)).toEqual({
      prose: "說明在前。",
      artifact: { kind: "html", content: "<html><body>doc</body></html>" },
    });
  });
});
