# tools/ApiDocFetcher/strategies/redoc.py
from __future__ import annotations

from strategies import Detection, DocNode, PageContent
from strategies.openapi_direct import OpenApiDirectStrategy


class RedocStrategy:
    """Redoc sites expose an OpenAPI spec URL — delegate to OpenApiDirectStrategy."""

    def __init__(self, detection: Detection):
        self._inner = OpenApiDirectStrategy(detection)

    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]:
        return self._inner.fetch_tree(url, cookies)

    def fetch_page(self, url: str, cookies: dict) -> PageContent:
        return self._inner.fetch_page(url, cookies)
