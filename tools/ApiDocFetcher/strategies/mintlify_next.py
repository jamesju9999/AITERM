# tools/ApiDocFetcher/strategies/mintlify_next.py
from __future__ import annotations
import json
import re
import yaml
from curl_cffi import requests
from strategies import Detection, DocNode, PageContent


def _extract_chunks(html: str) -> list[str]:
    """Return raw data strings from all __next_f.push calls."""
    return re.findall(
        r'self\.__next_f\.push\(\[\d+,(.+?)\]\)</script>',
        html,
        re.DOTALL,
    )


def _unescape_chunk(data: str) -> str:
    data = data.strip()
    if data.startswith('"') and data.endswith('"'):
        try:
            return json.loads(data)
        except (json.JSONDecodeError, ValueError):
            return data[1:-1]
    return data


class MintlifyNextStrategy:
    def __init__(self, detection: Detection):
        self._detection = detection

    def _fetch_html(self, url: str, cookies: dict) -> str:
        r = requests.get(url, impersonate="chrome120", timeout=30, cookies=cookies)
        return r.text

    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]:
        html = self._fetch_html(url, cookies)
        for data in _extract_chunks(html):
            content = _unescape_chunk(data)
            if '"title"' not in content or '"href"' not in content or '"items"' not in content:
                continue
            # Find "items": [ and extract the balanced array
            m = re.search(r'"items"\s*:\s*\[', content)
            if not m:
                continue
            bracket_start = m.end() - 1  # Position of [
            level = 0
            bracket_end = bracket_start
            for i, c in enumerate(content[bracket_start:]):
                if c == '[':
                    level += 1
                elif c == ']':
                    level -= 1
                    if level == 0:
                        bracket_end = bracket_start + i + 1
                        break
            try:
                items_str = content[bracket_start:bracket_end]
                raw = json.loads(items_str)
                return [_dict_to_node(item) for item in raw]
            except (json.JSONDecodeError, KeyError, ValueError):
                continue
        return []

    def fetch_page(self, url: str, cookies: dict) -> PageContent:
        html = self._fetch_html(url, cookies)
        specs: list[dict] = []
        for data in _extract_chunks(html):
            content = _unescape_chunk(data)
            if "openapi:" not in content:
                continue
            try:
                spec = yaml.safe_load(content)
                if isinstance(spec, dict) and "openapi" in spec:
                    specs.append(spec)
            except yaml.YAMLError:
                continue

        if specs:
            # Merge all specs: start from the largest, then fold in any paths/schemas
            # from other chunks that aren't already present (server may split spec across chunks).
            base = max(specs, key=lambda s: len(str(s)))
            merged_paths: dict = dict(base.get("paths", {}))
            merged_schemas: dict = dict(
                base.get("components", {}).get("schemas", {})
            )
            for other in specs:
                if other is base:
                    continue
                for path, item in (other.get("paths") or {}).items():
                    if path not in merged_paths:
                        merged_paths[path] = item
                for name, schema in (
                    other.get("components", {}).get("schemas") or {}
                ).items():
                    if name not in merged_schemas:
                        merged_schemas[name] = schema
            if merged_paths != base.get("paths"):
                import copy
                base = copy.deepcopy(base)
                base["paths"] = merged_paths
                if merged_schemas:
                    base.setdefault("components", {})["schemas"] = merged_schemas
            title = base.get("info", {}).get("title", url.split("/")[-1])
            return PageContent(title=title, openapi_spec=base, platform="mintlify-next")

        # No OpenAPI found — extract prose from main content only
        from bs4 import BeautifulSoup
        soup = BeautifulSoup(html, "html.parser")

        # Remove boilerplate and noise elements
        for tag in soup(["script", "style", "noscript", "nav", "header",
                          "footer", "aside", "svg", "img", "button"]):
            tag.decompose()

        # Try to find main content area
        main = (
            soup.find("main") or
            soup.find(attrs={"role": "main"}) or
            soup.find("article") or
            soup.find(class_=lambda c: c and any(
                kw in c for kw in ("content", "article", "prose", "main", "docs")
            )) or
            soup.body or
            soup
        )

        raw_lines = main.get_text(separator="\n", strip=True).splitlines()

        # Step 1: collapse runs of single-char lines into words
        # (happens when each character is in its own <span> for CSS animation)
        collapsed: list[str] = []
        i = 0
        while i < len(raw_lines):
            line = raw_lines[i].strip()
            if len(line) == 1 and line.isascii():
                chars = [line]
                j = i + 1
                while j < len(raw_lines) and len(raw_lines[j].strip()) <= 1:
                    c = raw_lines[j].strip()
                    if c:
                        chars.append(c)
                    j += 1
                if len(chars) >= 3:
                    # Join chars; insert space before uppercase after lowercase (CamelCase boundary)
                    word = chars[0]
                    for c in chars[1:]:
                        if c.isupper() and word and word[-1].islower():
                            word += " " + c
                        elif c == "(" and word and word[-1] != " ":
                            word += " " + c
                        else:
                            word += c
                    collapsed.append(word)
                    i = j
                    continue
            collapsed.append(raw_lines[i])
            i += 1

        # Step 2: deduplicate and remove blank-line runs
        seen: set[str] = set()
        deduped: list[str] = []
        blank_run = 0
        for line in collapsed:
            stripped = line.strip()
            if not stripped:
                blank_run += 1
                if blank_run <= 1:
                    deduped.append("")
            else:
                blank_run = 0
                if len(stripped) < 60 and stripped in seen:
                    continue
                seen.add(stripped)
                deduped.append(stripped)

        text = "\n".join(deduped).strip()
        title = soup.title.string if soup.title else url.split("/")[-1]
        return PageContent(title=title, raw_text=text, needs_ai=True, platform="mintlify-next")


def _dict_to_node(d: dict) -> DocNode:
    return DocNode(
        title=d.get("title", ""),
        href=d.get("href", ""),
        items=[_dict_to_node(c) for c in d.get("items", [])],
    )
