#!/usr/bin/env python3
"""
MarkItDown converter — stdin/stdout JSON line protocol.
Usage: python converter.py <file_path>

Stdout (exactly one line):
  {"type": "done", "markdown": "<converted text>"}
  {"type": "error", "message": "<error description>"}
"""
import sys
import json
import os

IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".tiff", ".tif"}


def image_fallback(file_path: str) -> str:
    """Extract basic image info via Pillow when MarkItDown returns empty content."""
    from PIL import Image
    from PIL.ExifTags import TAGS

    with Image.open(file_path) as img:
        lines = [
            f"# {os.path.basename(file_path)}",
            "",
            f"- **格式 / Format:** {img.format}",
            f"- **尺寸 / Size:** {img.width} × {img.height} px",
            f"- **色彩模式 / Mode:** {img.mode}",
        ]

        # Try to read EXIF data
        try:
            exif_data = img._getexif()  # type: ignore[attr-defined]
            if exif_data:
                lines.append("")
                lines.append("## EXIF")
                interesting = {
                    "Make", "Model", "Software", "DateTime", "DateTimeOriginal",
                    "ExposureTime", "FNumber", "ISOSpeedRatings", "FocalLength",
                    "Flash", "GPSInfo", "ImageDescription", "Artist", "Copyright",
                }
                for tag_id, value in exif_data.items():
                    tag_name = TAGS.get(tag_id, str(tag_id))
                    if tag_name in interesting and value:
                        lines.append(f"- **{tag_name}:** {value}")
        except Exception:
            pass

    return "\n".join(lines)


def main() -> None:
    if len(sys.argv) < 2:
        print(json.dumps({"type": "error", "message": "Usage: converter.py <file_path>"}, ensure_ascii=False), flush=True)
        sys.exit(1)

    file_path = sys.argv[1]
    try:
        from markitdown import MarkItDown
        md = MarkItDown()
        result = md.convert(file_path)
        markdown = result.text_content or ""

        # MarkItDown returns empty string for images when exiftool is not installed
        # and no LLM client is configured. Fall back to Pillow-based extraction.
        if not markdown.strip():
            ext = os.path.splitext(file_path)[1].lower()
            if ext in IMAGE_EXTENSIONS:
                try:
                    markdown = image_fallback(file_path)
                except Exception as img_err:
                    print(json.dumps({
                        "type": "error",
                        "message": (
                            f"圖片資訊讀取失敗：{img_err}\n"
                            "提示：安裝 exiftool 可取得完整 EXIF 資訊；"
                            "設定 LLM 視覺模型可取得圖片描述。"
                        ),
                    }, ensure_ascii=False), flush=True)
                    sys.exit(1)

        print(json.dumps({"type": "done", "markdown": markdown}, ensure_ascii=False), flush=True)
    except Exception as exc:  # noqa: BLE001
        print(json.dumps({"type": "error", "message": str(exc)}, ensure_ascii=False), flush=True)
        sys.exit(1)


if __name__ == "__main__":
    main()
