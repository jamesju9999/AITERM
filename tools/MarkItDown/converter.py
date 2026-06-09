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


def main() -> None:
    if len(sys.argv) < 2:
        print(json.dumps({"type": "error", "message": "Usage: converter.py <file_path>"}, ensure_ascii=False), flush=True)
        sys.exit(1)

    file_path = sys.argv[1]
    try:
        from markitdown import MarkItDown
        md = MarkItDown()
        result = md.convert(file_path)
        print(json.dumps({"type": "done", "markdown": result.text_content}, ensure_ascii=False), flush=True)
    except Exception as exc:  # noqa: BLE001
        print(json.dumps({"type": "error", "message": str(exc)}, ensure_ascii=False), flush=True)
        sys.exit(1)


if __name__ == "__main__":
    main()
