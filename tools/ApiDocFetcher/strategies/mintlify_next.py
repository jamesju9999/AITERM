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
            return bytes(data[1:-1], "utf-8").decode("unicode_escape")
        except Exception:
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
            # Return the largest spec (most complete)
            spec = max(specs, key=lambda s: len(str(s)))
            title = spec.get("info", {}).get("title", url.split("/")[-1])
            return PageContent(title=title, openapi_spec=spec, platform="mintlify-next")

        # No OpenAPI found — return raw text for AI fallback
        from bs4 import BeautifulSoup
        soup = BeautifulSoup(html, "html.parser")
        for tag in soup(["script", "style", "noscript"]):
            tag.decompose()
        text = soup.get_text(separator="\n", strip=True)
        title = soup.title.string if soup.title else url.split("/")[-1]
        return PageContent(title=title, raw_text=text, needs_ai=True, platform="mintlify-next")


def _dict_to_node(d: dict) -> DocNode:
    return DocNode(
        title=d.get("title", ""),
        href=d.get("href", ""),
        items=[_dict_to_node(c) for c in d.get("items", [])],
    )
