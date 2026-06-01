# tools/ApiDocFetcher/strategies/ai_generic.py
from __future__ import annotations

from curl_cffi import requests
from strategies import Detection, DocNode, PageContent
from bs4 import BeautifulSoup


class AiGenericStrategy:
    """Last-resort strategy: extract raw text for AI processing in Rust layer."""

    def __init__(self, detection: Detection):
        self._detection = detection

    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]:
        # Cannot determine tree structure without AI — return empty; UI will show single page
        return []

    def fetch_page(self, url: str, cookies: dict) -> PageContent:
        r = requests.get(url, impersonate="chrome120", timeout=30, cookies=cookies)
        soup = BeautifulSoup(r.text, "html.parser")
        for tag in soup(["script", "style", "noscript"]):
            tag.decompose()
        text = soup.get_text(separator="\n", strip=True)
        title = soup.title.string if soup.title else url.split("/")[-1]
        return PageContent(title=title, raw_text=text[:8000], needs_ai=True, platform="ai-generic")
