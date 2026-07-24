# tools/ApiDocFetcher/strategies/openapi_direct.py
from __future__ import annotations

import json
import re
import yaml
from urllib.parse import urlparse, urljoin, parse_qs, unquote
from curl_cffi import requests
from strategies import Detection, DocNode, PageContent


def _parse_spec(text: str) -> dict:
    """Parse *text* into an OpenAPI/Swagger spec dict, or raise ValueError.

    Guards against the common failure where the resolved "spec URL" actually
    serves an HTML page (Swagger UI shell, error page, ...): yaml.safe_load on
    HTML/CSS throws an ugly ScannerError, so we reject non-spec content cleanly
    and let the caller fall back."""
    text = text.strip()
    if not text or text[0] in "<":
        raise ValueError("content is HTML, not an OpenAPI spec")
    data = None
    try:
        data = json.loads(text)
    except Exception:
        try:
            data = yaml.safe_load(text)
        except Exception as exc:
            raise ValueError(f"content is neither JSON nor valid YAML: {exc}") from exc
    if not isinstance(data, dict) or not ("openapi" in data or "swagger" in data):
        raise ValueError("parsed content is not an OpenAPI spec")
    return data

OPENAPI_PROBE_PATHS = [
    "/openapi.json", "/swagger.json", "/openapi.yaml",
    "/api-docs", "/api/openapi.json", "/api/v3/openapi.json",
    "/v2/swagger.json", "/api/swagger.json",
]


def _resolve_spec_url(page_url: str, detection: Detection, cookies: dict) -> str:
    """Return a spec URL. The ?url=/?configUrl= query params are the most
    authoritative source for a Swagger UI page and are checked first (the
    detector's HTML-scraped spec_url can be wrong), then detection.spec_url,
    then page scripts / common probe paths."""
    # Swagger UI passes the spec via ?url=<spec> or ?configUrl=<config>. The
    # config resource is itself JSON that names the real spec(s) — resolve it.
    qs = parse_qs(urlparse(page_url).query)
    if qs.get("url"):
        return urljoin(page_url, unquote(qs["url"][0]))
    if qs.get("configUrl"):
        config_url = urljoin(page_url, unquote(qs["configUrl"][0]))
        try:
            cr = requests.get(config_url, impersonate="chrome120", timeout=10, cookies=cookies)
            cfg = json.loads(cr.text.strip())
            if isinstance(cfg, dict):
                if "openapi" in cfg or "swagger" in cfg:
                    return config_url  # configUrl already points at the spec
                if cfg.get("url"):
                    return urljoin(config_url, cfg["url"])
                urls = cfg.get("urls")
                if isinstance(urls, list) and urls and isinstance(urls[0], dict) and urls[0].get("url"):
                    return urljoin(config_url, urls[0]["url"])
        except Exception:
            pass

    if detection.spec_url:
        return detection.spec_url

    # Try to find spec URL from Swagger-initializer scripts or inline config
    try:
        r = requests.get(page_url, impersonate="chrome120", timeout=15, cookies=cookies)
        html = r.text

        # Look for swagger-initializer.js or similar external script with spec URL
        init_scripts = re.findall(r'<script[^>]+src=["\']([^"\']*(?:initializer|swagger-config)[^"\']*)["\']', html, re.IGNORECASE)
        base = page_url.rstrip("/").rsplit("/", 1)[0] if "/" in page_url else page_url
        for script_path in init_scripts:
            script_url = script_path if script_path.startswith("http") else base + "/" + script_path.lstrip("./")
            try:
                sr = requests.get(script_url, impersonate="chrome120", timeout=10, cookies=cookies)
                m = re.search(r'url\s*:\s*["\']([^"\']+)["\']', sr.text)
                if m:
                    found = m.group(1)
                    return found if found.startswith("http") else base + "/" + found.lstrip("./")
            except Exception:
                pass

        # Inline url: "..." pattern
        m = re.search(r'url\s*:\s*["\']([^"\']+\.(?:json|yaml))["\']', html)
        if m:
            found = m.group(1)
            return found if found.startswith("http") else base + found
    except Exception:
        pass

    # Probe common paths
    parsed = urlparse(page_url)
    base_root = f"{parsed.scheme}://{parsed.netloc}"
    for path in OPENAPI_PROBE_PATHS:
        probe = base_root + path
        try:
            r = requests.get(probe, impersonate="chrome120", timeout=8, cookies=cookies)
            if r.status_code == 200 and len(r.text) > 100:
                t = r.text.strip()[:200].lower()
                if "openapi" in t or "swagger" in t:
                    return probe
        except Exception:
            continue

    raise ValueError(f"Could not find OpenAPI spec URL for {page_url}")


class OpenApiDirectStrategy:
    def __init__(self, detection: Detection):
        self._detection = detection

    def _load_spec(self, page_url: str, cookies: dict) -> dict:
        url = _resolve_spec_url(page_url, self._detection, cookies)
        r = requests.get(url, impersonate="chrome120", timeout=30, cookies=cookies)
        r.raise_for_status()
        return _parse_spec(r.text)

    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]:
        spec = self._load_spec(url, cookies)
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
        spec = self._load_spec(url, cookies)
        title = spec.get("info", {}).get("title", "API Reference")
        return PageContent(title=title, openapi_spec=spec, platform="openapi-direct")
