# tools/ApiDocFetcher/strategies/docusaurus.py
from __future__ import annotations

import json
from urllib.parse import urlparse, urlunparse
from curl_cffi import requests
from strategies import Detection, DocNode, PageContent
from bs4 import BeautifulSoup

SIDEBAR_PATHS = ["/docs/sidebar.json", "/_docusaurus/sidebar.json", "/sidebar.json"]


class DocusaurusStrategy:
    def __init__(self, detection: Detection):
        self._detection = detection

    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]:
        # Extract base URL by removing trailing /docs path
        parsed = urlparse(url)
        path = parsed.path.rstrip("/")
        if path.endswith("/docs"):
            path = path[:-5]
        elif path == "/docs":
            path = ""
        base = urlunparse((parsed.scheme, parsed.netloc, path, "", "", ""))

        for path in SIDEBAR_PATHS:
            try:
                r = requests.get(base + path, impersonate="chrome120", timeout=10, cookies=cookies)
                if r.status_code == 200:
                    data = json.loads(r.text)
                    items = list(data.values())[0] if isinstance(data, dict) else data
                    return [_sidebar_item_to_node(item) for item in items]
            except Exception:
                continue
        return []

    def fetch_page(self, url: str, cookies: dict) -> PageContent:
        r = requests.get(url, impersonate="chrome120", timeout=30, cookies=cookies)
        soup = BeautifulSoup(r.text, "html.parser")
        article = soup.find("article") or soup.find("main") or soup.body
        for tag in (article or soup)(["script", "style"]):
            tag.decompose()
        text = (article or soup).get_text(separator="\n", strip=True) if article else ""
        title = soup.title.string if soup.title else url.split("/")[-1]
        return PageContent(title=title, raw_text=text, needs_ai=True, platform="docusaurus")


def _sidebar_item_to_node(item: dict) -> DocNode:
    if item.get("type") == "category":
        return DocNode(
            title=item.get("label", ""),
            href="",
            items=[_sidebar_item_to_node(c) for c in item.get("items", [])],
        )
    return DocNode(title=item.get("label", item.get("id", "")),
                   href=f"/docs/{item.get('id', '')}")
