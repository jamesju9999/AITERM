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
import urllib.request

IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".tiff", ".tif"}

DEFAULT_IMAGE_PROMPT = (
    "請完整抄寫這張圖片中的所有文字與數字，不得遺漏任何儲存格內容。\n\n"
    "規則：\n"
    "1. 表格：每一列、每一欄的數值都必須完整填入，禁止留空。若格子為空白，填入空字串即可。\n"
    "2. 數字：原樣抄寫，包含小數點、負號、千分位符號、貨幣符號等。\n"
    "3. 文字：原樣抄寫，保留換行與縮排。\n"
    "4. 格式：使用 Markdown 表格（若原始為表格），其他內容用適當的 Markdown 標記。\n"
    "5. 若圖片為照片（非文件），則詳細描述畫面內容。\n\n"
    "直接輸出內容，不要加任何說明或前言。"
)


def _read_image_as_base64(file_path: str) -> tuple[str, str]:
    """Return (base64_data, mime_type)."""
    mime_type, _ = mimetypes.guess_type(file_path)
    if not mime_type:
        mime_type = "image/jpeg"
    with open(file_path, "rb") as f:
        return base64.standard_b64encode(f.read()).decode("utf-8"), mime_type


def _http_post(url: str, payload: dict, headers: dict) -> dict:
    """Simple HTTP POST using only Python stdlib — no external packages needed."""
    data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    headers = {"Content-Type": "application/json", **headers}
    req = urllib.request.Request(url, data=data, headers=headers, method="POST")
    with urllib.request.urlopen(req, timeout=120) as resp:
        return json.loads(resp.read().decode("utf-8"))


def describe_image_openai_compatible(
    file_path: str, api_key: str, base_url: str, model: str, prompt: str
) -> str:
    """Call OpenAI / Ollama / OpenAI-compatible vision API using stdlib HTTP."""
    b64, mime_type = _read_image_as_base64(file_path)
    data_uri = f"data:{mime_type};base64,{b64}"

    url = base_url.rstrip("/") + "/chat/completions"
    headers = {}
    if api_key:
        headers["Authorization"] = f"Bearer {api_key}"

    payload = {
        "model": model,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": prompt},
                {"type": "image_url", "image_url": {"url": data_uri}},
            ],
        }],
        "max_tokens": 8192,
    }
    result = _http_post(url, payload, headers)
    return result["choices"][0]["message"]["content"] or ""


def describe_image_anthropic(
    file_path: str, api_key: str, base_url: str, model: str, prompt: str
) -> str:
    """Call Anthropic Messages API using stdlib HTTP."""
    b64, mime_type = _read_image_as_base64(file_path)
    if mime_type not in {"image/jpeg", "image/png", "image/gif", "image/webp"}:
        mime_type = "image/jpeg"

    url = (base_url.rstrip("/") if base_url else "https://api.anthropic.com") + "/v1/messages"
    headers = {
        "x-api-key": api_key,
        "anthropic-version": "2023-06-01",
    }
    payload = {
        "model": model,
        "max_tokens": 8192,
        "messages": [{
            "role": "user",
            "content": [
                {
                    "type": "image",
                    "source": {"type": "base64", "media_type": mime_type, "data": b64},
                },
                {"type": "text", "text": prompt},
            ],
        }],
    }
    result = _http_post(url, payload, headers)
    return result["content"][0]["text"] if result.get("content") else ""


def describe_image_with_ai(file_path: str) -> str | None:
    """Call the configured AI provider to describe the image. Returns None if not configured."""
    provider_type = os.environ.get("MARKITDOWN_LLM_PROVIDER_TYPE", "").lower()
    api_key = os.environ.get("MARKITDOWN_LLM_API_KEY", "")
    base_url = os.environ.get("MARKITDOWN_LLM_BASE_URL", "")
    model = os.environ.get("MARKITDOWN_LLM_MODEL", "")
    prompt = os.environ.get("MARKITDOWN_LLM_PROMPT", DEFAULT_IMAGE_PROMPT)

    if not provider_type or not model:
        return None

    if provider_type == "anthropic":
        return describe_image_anthropic(file_path, api_key, base_url, model, prompt)
    else:
        # openai / ollama / openai-compatible all share the same REST format
        if not base_url:
            base_url = (
                "http://localhost:11434/v1"
                if provider_type == "ollama"
                else "https://api.openai.com/v1"
            )
        return describe_image_openai_compatible(file_path, api_key, base_url, model, prompt)


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
