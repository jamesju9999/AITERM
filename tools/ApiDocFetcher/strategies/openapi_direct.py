# tools/ApiDocFetcher/strategies/openapi_direct.py
from __future__ import annotations

import json
import yaml
from curl_cffi import requests
from strategies import Detection, DocNode, PageContent


class OpenApiDirectStrategy:
    def __init__(self, detection: Detection):
        self._detection = detection

    def _load_spec(self, cookies: dict) -> dict:
        url = self._detection.spec_url
        r = requests.get(url, impersonate="chrome120", timeout=30, cookies=cookies)
        r.raise_for_status()
        text = r.text.strip()
        if text.startswith("{"):
            return json.loads(text)
        return yaml.safe_load(text)

    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]:
        spec = self._load_spec(cookies)
        paths = spec.get("paths", {})

        # Group endpoints by first tag; ungrouped go under "Other"
        groups: dict[str, list[DocNode]] = {}
        for path, path_item in paths.items():
            if not isinstance(path_item, dict):
                continue
            for method in ("get", "post", "put", "patch", "delete"):
                op = path_item.get(method)
                if not op:
                    continue
                tags = op.get("tags", ["Other"])
                tag = tags[0] if tags else "Other"
                groups.setdefault(tag, []).append(
                    DocNode(title=f"{method.upper()} {path}", href=path)
                )

        return [DocNode(title=tag, href="", items=items)
                for tag, items in groups.items()]

    def fetch_page(self, url: str, cookies: dict) -> PageContent:
        spec = self._load_spec(cookies)
        title = spec.get("info", {}).get("title", "API Reference")
        return PageContent(title=title, openapi_spec=spec, platform="openapi-direct")
