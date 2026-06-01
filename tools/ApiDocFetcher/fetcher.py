#!/usr/bin/env python3
# tools/ApiDocFetcher/fetcher.py
"""
ApiDocFetcher CLI.

Commands:
  detect  --url URL [--cookies COOKIE_STR]
  tree    --url URL [--cookies COOKIE_STR]
  extract --url URL --pages JSON_ARRAY --output DIR [--merge] [--keep JSON] [--cookies COOKIE_STR]

All output is line-delimited JSON to stdout.
"""
import argparse
import json
import os
import sys

# Ensure strategies package is importable when run from any working dir
sys.path.insert(0, os.path.dirname(__file__))

from detector import detect
from strategies import get_strategy, KeepOptions
from converter import openapi_to_markdown


def _parse_cookies(cookie_str: str) -> dict:
    if not cookie_str:
        return {}
    result = {}
    for part in cookie_str.split(";"):
        if "=" in part:
            k, v = part.strip().split("=", 1)
            result[k.strip()] = v.strip()
    return result


def _emit(obj: dict) -> None:
    print(json.dumps(obj, ensure_ascii=False), flush=True)


def _node_to_dict(node) -> dict:
    return {"title": node.title, "href": node.href,
            "items": [_node_to_dict(c) for c in node.items]}


def cmd_detect(args) -> None:
    cookies = _parse_cookies(args.cookies)
    result = detect(args.url, cookies)
    _emit({"type": "detected", "platform": result.platform,
           "confidence": result.confidence, "spec_url": result.spec_url})


def cmd_tree(args) -> None:
    cookies = _parse_cookies(args.cookies)
    detection = detect(args.url, cookies)
    _emit({"type": "detected", "platform": detection.platform,
           "confidence": detection.confidence})
    strategy = get_strategy(detection)
    nodes = strategy.fetch_tree(args.url, cookies)
    _emit({"type": "tree", "data": [_node_to_dict(n) for n in nodes]})


def cmd_extract(args) -> None:
    cookies = _parse_cookies(args.cookies)
    pages: list[dict] = json.loads(args.pages)
    keep = KeepOptions.from_dict(json.loads(args.keep)) if args.keep else KeepOptions()

    detection = detect(args.url, cookies)
    _emit({"type": "detected", "platform": detection.platform,
           "confidence": detection.confidence})
    strategy = get_strategy(detection)

    total = len(pages)
    markdowns: list[tuple[dict, str]] = []

    for i, page in enumerate(pages):
        base = args.url.rstrip("/").split("/docs")[0]
        href = page.get("href", "")
        page_url = href if href.startswith("http") else base + href

        _emit({"type": "progress", "current": i + 1, "total": total,
               "page": page.get("title", href)})
        try:
            content = strategy.fetch_page(page_url, cookies)
            if content.needs_ai:
                # Signal to Rust that AI processing is needed for this page
                _emit({"type": "needs_ai", "title": content.title,
                       "raw_text": content.raw_text or ""})
                md = f"# {content.title}\n\n> ⚠ AI processing required\n\n{content.raw_text or ''}"
            elif content.openapi_spec:
                md = openapi_to_markdown(content.openapi_spec, keep)
            else:
                md = f"# {content.title}\n\n{content.raw_text or ''}"

            size_kb = len(md.encode()) // 1024
            _emit({"type": "log", "level": "info",
                   "message": f"✓ {page.get('title', href)} ({size_kb}KB)"})
            markdowns.append((page, md))
        except Exception as exc:
            _emit({"type": "log", "level": "error",
                   "message": f"✗ Failed: {page.get('title', href)}: {exc}"})

    os.makedirs(args.output, exist_ok=True)
    files: list[str] = []

    if args.merge:
        out_path = os.path.join(args.output, "api-docs.md")
        with open(out_path, "w", encoding="utf-8") as f:
            f.write("\n\n---\n\n".join(md for _, md in markdowns))
        files.append(out_path)
    else:
        for page, md in markdowns:
            slug = page.get("href", "page").strip("/").split("/")[-1] or "page"
            out_path = os.path.join(args.output, f"{slug}.md")
            with open(out_path, "w", encoding="utf-8") as f:
                f.write(md)
            files.append(out_path)

    _emit({"type": "done", "files": files})


def main() -> None:
    parser = argparse.ArgumentParser(description="ApiDocFetcher CLI")
    sub = parser.add_subparsers(dest="cmd", required=True)

    for name in ("detect", "tree"):
        p = sub.add_parser(name)
        p.add_argument("--url", required=True)
        p.add_argument("--cookies", default="")

    p_ex = sub.add_parser("extract")
    p_ex.add_argument("--url", required=True)
    p_ex.add_argument("--pages", required=True, help="JSON array of {title, href}")
    p_ex.add_argument("--output", required=True)
    p_ex.add_argument("--merge", action="store_true")
    p_ex.add_argument("--keep", default="", help="JSON KeepOptions")
    p_ex.add_argument("--cookies", default="")

    args = parser.parse_args()
    {"detect": cmd_detect, "tree": cmd_tree, "extract": cmd_extract}[args.cmd](args)


if __name__ == "__main__":
    main()
