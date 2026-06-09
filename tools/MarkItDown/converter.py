#!/usr/bin/env python3
"""
MarkItDown converter — stdin/stdout JSON line protocol.
Usage: python converter.py <file_path>

Stdout (exactly one line):
  {"type": "done", "markdown": "<converted text>"}
  {"type": "error", "message": "<error description>"}

Environment variables (optional, for image vision):
  MARKITDOWN_LLM_PROVIDER_TYPE  openai | anthropic | ollama | openai-compatible
  MARKITDOWN_LLM_API_KEY        API key (not needed for Ollama)
  MARKITDOWN_LLM_BASE_URL       Override base URL
  MARKITDOWN_LLM_MODEL          Model name (e.g. gpt-4o, claude-3-5-sonnet-20241022)
  MARKITDOWN_LLM_PROMPT         Custom prompt for image description
"""
import sys
import json
import os
import base64
import mimetypes

IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".tiff", ".tif"}

DEFAULT_IMAGE_PROMPT = (
    "請完整描述這張圖片的內容。"
    "如果圖片中有文字（包括截圖、掃描文件、投影片、表格等），請原文抄寫所有文字內容，並保持原始格式（換行、縮排、表格結構等）。"
    "如果是純圖片或照片，請詳細描述畫面中的人物、物件、場景與任何可見文字。"
    "輸出使用 Markdown 格式。"
)


def _read_image_as_base64(file_path: str) -> tuple[str, str]:
    """Return (base64_data, mime_type)."""
    mime_type, _ = mimetypes.guess_type(file_path)
    if not mime_type:
        mime_type = "image/jpeg"
    with open(file_path, "rb") as f:
        return base64.standard_b64encode(f.read()).decode("utf-8"), mime_type


def describe_image_openai(file_path: str, api_key: str, base_url: str, model: str, prompt: str) -> str:
    from openai import OpenAI
    client = OpenAI(api_key=api_key or "ollama", base_url=base_url)
    b64, mime_type = _read_image_as_base64(file_path)
    data_uri = f"data:{mime_type};base64,{b64}"
    response = client.chat.completions.create(
        model=model,
        messages=[{
            "role": "user",
            "content": [
                {"type": "text", "text": prompt},
                {"type": "image_url", "image_url": {"url": data_uri}},
            ],
        }],
        max_tokens=4096,
    )
    return response.choices[0].message.content or ""


def describe_image_anthropic(file_path: str, api_key: str, base_url: str, model: str, prompt: str) -> str:
    import anthropic
    kwargs: dict = {"api_key": api_key}
    if base_url:
        kwargs["base_url"] = base_url
    client = anthropic.Anthropic(**kwargs)
    b64, mime_type = _read_image_as_base64(file_path)
    # Anthropic only accepts specific image media types
    if mime_type not in {"image/jpeg", "image/png", "image/gif", "image/webp"}:
        mime_type = "image/jpeg"
    response = client.messages.create(
        model=model,
        max_tokens=4096,
        messages=[{
            "role": "user",
            "content": [
                {
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": mime_type,
                        "data": b64,
                    },
                },
                {"type": "text", "text": prompt},
            ],
        }],
    )
    return response.content[0].text if response.content else ""


def describe_image_with_ai(file_path: str) -> str | None:
    """Try to describe image using configured AI provider. Returns None if not configured."""
    provider_type = os.environ.get("MARKITDOWN_LLM_PROVIDER_TYPE", "").lower()
    api_key = os.environ.get("MARKITDOWN_LLM_API_KEY", "")
    base_url = os.environ.get("MARKITDOWN_LLM_BASE_URL", "")
    model = os.environ.get("MARKITDOWN_LLM_MODEL", "")
    prompt = os.environ.get("MARKITDOWN_LLM_PROMPT", DEFAULT_IMAGE_PROMPT)

    if not provider_type or not model:
        return None

    try:
        if provider_type == "anthropic":
            return describe_image_anthropic(file_path, api_key, base_url, model, prompt)
        else:
            # openai / ollama / openai-compatible all use OpenAI client format
            if not base_url:
                if provider_type == "ollama":
                    base_url = "http://localhost:11434/v1"
                else:
                    base_url = "https://api.openai.com/v1"
            return describe_image_openai(file_path, api_key, base_url, model, prompt)
    except ImportError as e:
        # Python package not yet installed — pip install may still be in progress
        # or failed silently. Raise a clear message instead of a raw ImportError.
        pkg = str(e).replace("No module named ", "").strip("'\"")
        raise RuntimeError(
            f"缺少 Python 套件：{pkg}\n"
            f"請重新嘗試一次（app 會自動安裝），或手動執行：\n"
            f"  python3 -m pip install {pkg}"
        ) from e


def image_metadata_fallback(file_path: str) -> str:
    """Extract basic image info via Pillow as last-resort fallback."""
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
        try:
            exif_data = img._getexif()  # type: ignore[attr-defined]
            if exif_data:
                interesting = {
                    "Make", "Model", "Software", "DateTime", "DateTimeOriginal",
                    "ExposureTime", "FNumber", "ISOSpeedRatings", "FocalLength",
                    "Flash", "GPSInfo", "ImageDescription", "Artist", "Copyright",
                }
                exif_lines = []
                for tag_id, value in exif_data.items():
                    tag_name = TAGS.get(tag_id, str(tag_id))
                    if tag_name in interesting and value:
                        exif_lines.append(f"- **{tag_name}:** {value}")
                if exif_lines:
                    lines.append("")
                    lines.append("## EXIF")
                    lines.extend(exif_lines)
        except Exception:
            pass

    return "\n".join(lines)


def main() -> None:
    if len(sys.argv) < 2:
        print(json.dumps({"type": "error", "message": "Usage: converter.py <file_path>"}, ensure_ascii=False), flush=True)
        sys.exit(1)

    file_path = sys.argv[1]
    ext = os.path.splitext(file_path)[1].lower()
    is_image = ext in IMAGE_EXTENSIONS

    try:
        # For images, prefer AI vision over MarkItDown (which returns empty without exiftool)
        if is_image:
            ai_result = describe_image_with_ai(file_path)
            if ai_result is not None:
                print(json.dumps({"type": "done", "markdown": ai_result.strip()}, ensure_ascii=False), flush=True)
                return

        from markitdown import MarkItDown
        md = MarkItDown()
        result = md.convert(file_path)
        markdown = result.text_content or ""

        # MarkItDown returns empty for images without exiftool — use Pillow metadata fallback
        if not markdown.strip() and is_image:
            markdown = image_metadata_fallback(file_path)

        print(json.dumps({"type": "done", "markdown": markdown}, ensure_ascii=False), flush=True)

    except Exception as exc:  # noqa: BLE001
        print(json.dumps({"type": "error", "message": str(exc)}, ensure_ascii=False), flush=True)
        sys.exit(1)


if __name__ == "__main__":
    main()
