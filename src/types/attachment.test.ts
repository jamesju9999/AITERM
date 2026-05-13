import { describe, it, expect } from "vitest";
import { buildContentParts, contentToDisplayString } from "./attachment";
import type { Attachment } from "./attachment";

const imageAtt: Attachment = {
  id: "1", kind: "image", name: "photo.png",
  mimeType: "image/png", data: "data:image/png;base64,abc", previewUrl: "data:image/png;base64,abc",
};
const textAtt: Attachment = {
  id: "2", kind: "text", name: "notes.md", mimeType: "text/markdown", data: "# hello",
};
const binaryAtt: Attachment = {
  id: "3", kind: "binary", name: "archive.zip",
  mimeType: "application/zip", data: "/Users/x/archive.zip",
};

describe("buildContentParts", () => {
  it("text only → single text part", () => {
    const parts = buildContentParts("hello", []);
    expect(parts).toEqual([{ type: "text", text: "hello" }]);
  });

  it("image attachment → image_url part", () => {
    const parts = buildContentParts("", [imageAtt]);
    expect(parts[0]).toEqual({ type: "image_url", image_url: { url: "data:image/png;base64,abc" } });
  });

  it("text file → text part with filename header", () => {
    const parts = buildContentParts("", [textAtt]);
    expect(parts[0].type).toBe("text");
    expect((parts[0] as { type: "text"; text: string }).text).toContain("[notes.md]");
    expect((parts[0] as { type: "text"; text: string }).text).toContain("# hello");
  });

  it("binary → path as text part", () => {
    const parts = buildContentParts("", [binaryAtt]);
    expect(parts[0]).toEqual({ type: "text", text: "/Users/x/archive.zip" });
  });
});

describe("contentToDisplayString", () => {
  it("string passthrough", () => {
    expect(contentToDisplayString("hello")).toBe("hello");
  });

  it("array → extracts text parts only", () => {
    const result = contentToDisplayString([
      { type: "text", text: "describe" },
      { type: "image_url", image_url: { url: "data:..." } },
    ]);
    expect(result).toBe("describe");
  });
});
