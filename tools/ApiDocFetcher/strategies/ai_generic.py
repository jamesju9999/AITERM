# tools/ApiDocFetcher/strategies/ai_generic.py
"""General-purpose fallback strategy.

Used when no specific platform (OpenAPI/Swagger/Redoc/Mintlify/Docusaurus)
matched. Rather than giving up with a single page, it crawls the site into a
browsable tree (fetch_tree) and extracts each page's main content for the Rust
AI layer to clean up into API-doc markdown (fetch_page).
"""
from __future__ import annotations

from collections import OrderedDict
from urllib.parse import urlparse, urljoin, urldefrag
from curl_cffi import requests
from strategies import Detection, DocNode, PageContent
from bs4 import BeautifulSoup

# Path segments that typically root a documentation section. When the entry URL
# contains one, the crawl is scoped to links under it so we don't wander into
# the marketing site.
DOC_ROOTS = {
    "docs", "doc", "api", "apis", "reference", "ref", "guide", "guides",
    "documentation", "developer", "developers", "manual", "help", "sdk",
}

# Non-page resources we never enqueue.
SKIP_EXT = (
    ".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".ico", ".css", ".js",
    ".pdf", ".zip", ".gz", ".tar", ".mp4", ".webm", ".woff", ".woff2", ".ttf",
    ".xml", ".rss",
)

MAX_PAGES = 60        # hard cap on nodes in the returned tree
MAX_FETCHES = 20      # hard cap on network requests during discovery
RICH_NAV = 8          # entry links at/above this => nav is complete, stop crawling
FETCH_TIMEOUT = 15


def _norm(url: str) -> str:
    """Drop fragment and a trailing slash so the same page dedups to one key."""
    url, _ = urldefrag(url)
    parsed = urlparse(url)
    if url.endswith("/") and len(parsed.path) > 1:
        url = url[:-1]
    return url


def _slug_title(url: str) -> str:
    parsed = urlparse(url)
    slug = parsed.path.rstrip("/").split("/")[-1] or parsed.netloc
    return slug.replace("-", " ").replace("_", " ").strip() or url


def _doc_prefix(url: str) -> str:
    """Path prefix that scopes the crawl to the docs section."""
    parts = [p for p in urlparse(url).path.split("/") if p]
    acc = ""
    for seg in parts:
        acc += "/" + seg
        if seg.lower() in DOC_ROOTS:
            return acc
    # No docs-root segment: scope to the entry's parent directory (or whole domain).
    if len(parts) > 1:
        return "/" + "/".join(parts[:-1])
    return ""


class AiGenericStrategy:
    """General fallback: crawl an unknown doc site into a tree + extract pages."""

    def __init__(self, detection: Detection):
        self._detection = detection

    # -- tree ------------------------------------------------------------------
    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]:
        parsed = urlparse(url)
        origin = f"{parsed.scheme}://{parsed.netloc}"
        prefix = _doc_prefix(url)
        entry = _norm(url)

        titles: "OrderedDict[str, str]" = OrderedDict()

        def add(abs_url: str, title: str) -> None:
            if abs_url in titles:
                # Upgrade a slug-derived title once we see real anchor text.
                if title and titles[abs_url] == _slug_title(abs_url):
                    titles[abs_url] = title
                return
            if len(titles) < MAX_PAGES:
                titles[abs_url] = title or _slug_title(abs_url)

        # The entry page is always selectable.
        add(entry, _slug_title(entry))

        # Depth 1: the entry page's own nav usually lists every doc page.
        entry_links = self._collect_links(entry, cookies, parsed.netloc, prefix)
        for u, ttl in entry_links:
            add(u, ttl)

        # Thin nav (JS-rendered or sparse) => crawl the discovered pages one level
        # deeper to find more, bounded by MAX_FETCHES.
        if len(titles) < RICH_NAV:
            fetches = 1  # counting the entry fetch
            for u, _ in entry_links:
                if fetches >= MAX_FETCHES or len(titles) >= MAX_PAGES:
                    break
                for cu, ct in self._collect_links(u, cookies, parsed.netloc, prefix):
                    add(cu, ct)
                fetches += 1

        nodes = [
            DocNode(title=ttl, href=(u[len(origin):] if u.startswith(origin) else u))
            for u, ttl in titles.items()
        ]
        return _group_tree(nodes, prefix)

    def _collect_links(self, page_url: str, cookies: dict, netloc: str,
                       prefix: str) -> list[tuple[str, str]]:
        """Return [(normalized_abs_url, anchor_title)] for same-domain doc links
        on *page_url*. Silent on any fetch/parse error (best-effort discovery)."""
        try:
            r = requests.get(page_url, impersonate="chrome120",
                             timeout=FETCH_TIMEOUT, cookies=cookies)
            if r.status_code != 200:
                return []
            soup = BeautifulSoup(r.text, "html.parser")
        except Exception:
            return []

        out: list[tuple[str, str]] = []
        seen: set[str] = set()
        for a in soup.find_all("a", href=True):
            href = a["href"].strip()
            if not href or href.startswith(("#", "mailto:", "tel:", "javascript:")):
                continue
            absu = _norm(urljoin(page_url, href))
            p = urlparse(absu)
            if p.scheme not in ("http", "https") or p.netloc != netloc:
                continue
            if prefix and not p.path.startswith(prefix):
                continue
            if p.path.lower().endswith(SKIP_EXT):
                continue
            if absu in seen:
                continue
            seen.add(absu)
            title = " ".join(a.get_text(" ", strip=True).split())[:120]
            out.append((absu, title))
        return out

    # -- page ------------------------------------------------------------------
    def fetch_page(self, url: str, cookies: dict) -> PageContent:
        r = requests.get(url, impersonate="chrome120", timeout=30, cookies=cookies)
        soup = BeautifulSoup(r.text, "html.parser")
        title = (soup.title.string.strip()
                 if soup.title and soup.title.string else _slug_title(url))
        text = _main_text(soup)
        return PageContent(title=title, raw_text=text[:16000],
                           needs_ai=True, platform="ai-generic")


def _main_text(soup: BeautifulSoup) -> str:
    """Extract the page's primary content, dropping nav/aside/header/footer
    boilerplate so the AI sees the actual documentation."""
    for tag in soup(["script", "style", "noscript", "svg"]):
        tag.decompose()
    main = (soup.find("main") or soup.find("article")
            or soup.find(attrs={"role": "main"}))
    if main is None:
        for sel in ("nav", "aside", "header", "footer"):
            for tag in soup.find_all(sel):
                tag.decompose()
        main = soup.body or soup
    return main.get_text(separator="\n", strip=True)


def _group_tree(nodes: list[DocNode], prefix: str) -> list[DocNode]:
    """Group flat page nodes by the first path segment after the doc prefix into
    a shallow browsable tree. Pages sitting directly under the prefix stay at the
    top level. Category nodes carry an empty href (the UI treats any node with
    children as a non-selectable container)."""
    groups: "OrderedDict[str, DocNode]" = OrderedDict()
    top: list[DocNode] = []
    plen = len(prefix)
    for n in nodes:
        # n.href is site-relative (or absolute); take the path portion.
        path = urlparse(n.href if n.href.startswith("http") else "http://_" + n.href).path
        rest = path[plen:] if prefix and path.startswith(prefix) else path
        segs = [s for s in rest.split("/") if s]
        if len(segs) <= 1:
            top.append(n)
            continue
        key = segs[0]
        if key not in groups:
            groups[key] = DocNode(title=key.replace("-", " ").replace("_", " "), href="")
        groups[key].items.append(n)
    return top + list(groups.values())
