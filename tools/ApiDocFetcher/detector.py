from __future__ import annotations

import re
from urllib.parse import urljoin
from curl_cffi import requests
from strategies import Detection

OPENAPI_PROBE_PATHS = [
    "/openapi.json",
    "/swagger.json",
    "/openapi.yaml",
    "/api-docs",
    "/api/openapi.json",
]


def detect(url: str, cookies: dict) -> Detection:
    """Probe *url* and return which doc platform it uses."""
    base = url.rstrip("/").rsplit("/docs", 1)[0] if "/docs" in url else url.rstrip("/")

    # 1. Try common OpenAPI spec paths
    for path in OPENAPI_PROBE_PATHS:
        probe = base + path
        try:
            r = requests.get(probe, impersonate="chrome120", timeout=10, cookies=cookies)
            if r.status_code == 200 and _looks_like_openapi(r.text):
                return Detection(platform="openapi-direct", confidence="high", spec_url=probe)
        except Exception:
            continue

    # 2. Fetch the page and inspect HTML
    try:
        r = requests.get(url, impersonate="chrome120", timeout=30, cookies=cookies)
        html = r.text
    except Exception:
        return Detection(platform="ai-generic", confidence="low")

    if "__next_f" in html:
        # Next.js App Router / Mintlify / FumaDocs site
        confidence = "high" if "openapi:" in html else "medium"
        return Detection(platform="mintlify-next", confidence=confidence)

    if "swagger-ui" in html.lower() or "SwaggerUIBundle" in html:
        spec_url = _extract_swagger_spec_url(html, url)
        return Detection(platform="swagger-ui", confidence="high", spec_url=spec_url)

    if re.search(r"<redoc[\s>]", html, re.IGNORECASE) or "ReDoc.init" in html:
        spec_url = _extract_redoc_spec_url(html, url)
        return Detection(platform="redoc", confidence="high", spec_url=spec_url)

    if "__docusaurus" in html or "docusaurus" in html:
        return Detection(platform="docusaurus", confidence="medium")

    return Detection(platform="ai-generic", confidence="low")


def _looks_like_openapi(text: str) -> bool:
    t = text.strip()[:200].lower()
    return "openapi" in t or "swagger" in t


def _extract_swagger_spec_url(html: str, page_url: str) -> str | None:
    for pat in [
        r'url\s*:\s*["\']([^"\']+\.(?:json|yaml))["\']',
        r'SwaggerUIBundle\b[^)]*?url\s*:\s*["\']([^"\']+)["\']',
    ]:
        m = re.search(pat, html)
        if m:
            # urljoin resolves absolute ("/api/x") and relative paths correctly
            # against the page URL; plain concatenation produced ".../index.html/api/x".
            return urljoin(page_url, m.group(1))
    return None


def _extract_redoc_spec_url(html: str, page_url: str) -> str | None:
    m = re.search(r'<redoc[^>]+spec-url=["\']([^"\']+)["\']', html, re.IGNORECASE)
    if m:
        return urljoin(page_url, m.group(1))
    m = re.search(r'ReDoc\.init\(["\']([^"\']+)["\']', html)
    if m:
        return urljoin(page_url, m.group(1))
    return None
