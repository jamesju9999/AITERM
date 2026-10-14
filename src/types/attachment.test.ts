import { describe, it, expect, vi, beforeEach } from "vitest";

// jsdom 沒有 canvas，真正的轉檔在 App 裡驗證；這裡只測「誰要轉、轉失敗怎麼辦」。
vi.mock("../lib/imageToPng", () => ({ convertToPng: vi.fn() }));

import { buildContentParts, contentToDisplayString, readFileAsAttachment, UnsupportedImageError } from "./attachment";
import { convertToPng } from "../lib/imageToPng";
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

describe("readFileAsAttachment：各家只保證收 jpeg／png／gif／webp，其他圖片轉成 PNG", () => {
  beforeEach(() => {
    vi.mocked(convertToPng).mockReset();
  });

  it.each(["image/png", "image/jpeg", "image/gif", "image/webp"])("%s 原樣讀取、不轉檔", async (type) => {
    const att = await readFileAsAttachment(new File(["x"], "a.img", { type }));
    expect(convertToPng).not.toHaveBeenCalled();
    expect(att.mimeType).toBe(type);
    expect(att.data.startsWith(`data:${type};base64,`)).toBe(true);
  });

  it.each(["image/bmp", "image/svg+xml", "image/tiff", "image/heic"])("%s 轉成 PNG，檔名不變", async (type) => {
    vi.mocked(convertToPng).mockResolvedValue("data:image/png;base64,UE5H");
    const file = new File(["x"], "pic.raw", { type });
    const att = await readFileAsAttachment(file);
    expect(convertToPng).toHaveBeenCalledWith(file);
    expect(att).toMatchObject({
      kind: "image", name: "pic.raw", mimeType: "image/png",
      data: "data:image/png;base64,UE5H", previewUrl: "data:image/png;base64,UE5H",
    });
  });

  it("解不開的圖片丟出 UnsupportedImageError（呼叫端據此提示使用者）", async () => {
    vi.mocked(convertToPng).mockRejectedValue(new Error("decode failed"));
    await expect(readFileAsAttachment(new File(["x"], "x.heic", { type: "image/heic" })))
      .rejects.toBeInstanceOf(UnsupportedImageError);
  });
});
