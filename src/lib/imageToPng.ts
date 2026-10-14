/**
 * 用 WebView 自己的解碼器把圖片轉成 PNG data URI。
 * 各家 AI 只保證收 jpeg／png／gif／webp，其他格式（bmp、svg、tiff，以及 macOS 上的 heic）先轉檔。
 * 解不開時丟錯，呼叫端會提示使用者「格式無法讀取」。
 */
export async function convertToPng(file: Blob): Promise<string> {
  const url = URL.createObjectURL(file);
  try {
    const img = new Image();
    img.src = url;
    await img.decode();
    // 沒有固有尺寸的 SVG 有些引擎回報 0，給一個合理的預設。
    const width = img.naturalWidth || 1024;
    const height = img.naturalHeight || 1024;
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas 2d context unavailable");
    ctx.drawImage(img, 0, 0, width, height);
    const data = canvas.toDataURL("image/png");
    // 畫布壞掉時 toDataURL 會回 "data:,"。
    if (!data.startsWith("data:image/png")) throw new Error("PNG encoding failed");
    return data;
  } finally {
    URL.revokeObjectURL(url);
  }
}
