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

# ── Dependency check ──────────────────────────────────────────────────────────
# Verify required third-party packages before importing anything else so we can
# emit a structured JSON error (instead of a raw traceback) when pip install
# failed (e.g. Ubuntu "externally managed environment" silently rejecting it).
_REQUIRED = {
    "curl_cffi": "curl_cffi>=0.7.0",
    "yaml":      "pyyaml>=6.0",
    "bs4":       "beautifulsoup4>=4.12",
}

def _check_deps() -> None:
    missing = []
    for module, pkg in _REQUIRED.items():
        try:
            __import__(module)
        except ImportError:
            missing.append(pkg)
    if missing:
        msg = (
            "Missing Python packages: " + ", ".join(missing) + ". "
            "Run: pip install " + " ".join(missing)
        )
        print(json.dumps({"type": "error", "message": msg}), flush=True)
        sys.exit(1)

_check_deps()
# ─────────────────────────────────────────────────────────────────────────────

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
    try:
        nodes = strategy.fetch_tree(args.url, cookies)
    except Exception as exc:
        # A platform strategy can fail on a real-world quirk (e.g. a Swagger UI
        # page whose spec URL actually serves HTML). Don't crash the whole tool —
        # fall back to the generic crawler so the user still gets a tree.
        _emit({"type": "log", "level": "warn",
               "message": f"{detection.platform} strategy failed ({exc}); using generic crawler"})
        from strategies.ai_generic import AiGenericStrategy
        nodes = AiGenericStrategy(detection).fetch_tree(args.url, cookies)
    _emit({"type": "tree", "data": [_node_to_dict(n) for n in nodes]})


def cmd_extract(args) -> None:
    from urllib.parse import urlparse
    cookies = _parse_cookies(args.cookies)
    pages: list = json.loads(args.pages)
    keep = KeepOptions.from_dict(json.loads(args.keep)) if args.keep else KeepOptions()

    detection = detect(args.url, cookies)
    _emit({"type": "detected", "platform": detection.platform,
           "confidence": detection.confidence})
    strategy = get_strategy(detection)

    total = len(pages)
    markdowns: list[tuple[str, str]] = []

    _parsed = urlparse(args.url)
    base = f"{_parsed.scheme}://{_parsed.netloc}"

    for i, page in enumerate(pages):
        if isinstance(page, dict):
            href = page.get("href", "")
            page_title = page.get("title", href)
        else:
            href = str(page)
            page_title = href.rstrip("/").split("/")[-1] or href
        page_url = href if href.startswith("http") else base + href

        _emit({"type": "progress", "current": i + 1, "total": total,
               "page": page_title})
        try:
            try:
                content = strategy.fetch_page(page_url, cookies)
            except Exception as exc:
                # Same resilience as the tree path: if the detected strategy
                # can't render this page, fall back to the generic extractor.
                _emit({"type": "log", "level": "warn",
                       "message": f"{detection.platform} failed on {page_title} ({exc}); using generic extractor"})
                from strategies.ai_generic import AiGenericStrategy
                content = AiGenericStrategy(detection).fetch_page(page_url, cookies)
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
                   "message": f"✓ {page_title} ({size_kb}KB)"})
            markdowns.append((href, md))
        except Exception as exc:
            _emit({"type": "log", "level": "error",
                   "message": f"✗ Failed: {page_title}: {exc}"})

    os.makedirs(args.output, exist_ok=True)
    files: list[str] = []

    if args.merge:
        out_path = os.path.join(args.output, "api-docs.md")
        # Strip trailing --- from each section to avoid double separators when joining
        def _strip_trailing_sep(md: str) -> str:
            s = md.rstrip()
            if s.endswith("---"):
                s = s[:-3].rstrip()
            return s
        with open(out_path, "w", encoding="utf-8") as f:
            f.write("\n\n---\n\n".join(_strip_trailing_sep(md) for _, md in markdowns))
        files.append(out_path)
    else:
        for href, md in markdowns:
            slug = href.strip("/").split("/")[-1] or "page"
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
