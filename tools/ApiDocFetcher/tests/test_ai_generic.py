import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from unittest.mock import patch
from tests.conftest import make_response
from strategies import Detection
from strategies.ai_generic import AiGenericStrategy, _doc_prefix, _group_tree, _main_text
from strategies import DocNode
from bs4 import BeautifulSoup


def _mock(url_map):
    def side_effect(url, **kwargs):
        return url_map.get(url, make_response(404, ""))
    return side_effect


ENTRY = "https://example.com/docs"

# A landing page whose sidebar nav lists the whole doc set (the common case).
RICH_HTML = """
<html><head><title>Docs</title></head><body>
<nav>
  <a href="/docs/intro">Introduction</a>
  <a href="/docs/auth/login">Login</a>
  <a href="/docs/auth/tokens">Tokens</a>
  <a href="/docs/api/users">Users API</a>
  <a href="/docs/api/orders">Orders API</a>
  <a href="/docs/api/orders#section">Orders anchor</a>
  <a href="/docs/guide/quickstart">Quickstart</a>
  <a href="/docs/guide/errors">Errors</a>
  <a href="https://twitter.com/x">External</a>
  <a href="/pricing">Marketing</a>
  <a href="/docs/logo.png">asset</a>
</nav>
<main><h1>Welcome</h1><p>Body</p></main>
</body></html>
"""


def test_doc_prefix_detects_docs_root():
    assert _doc_prefix("https://example.com/docs/auth/login") == "/docs"
    assert _doc_prefix("https://example.com/reference/api") == "/reference"


def test_doc_prefix_falls_back_to_parent_dir():
    assert _doc_prefix("https://example.com/foo/bar") == "/foo"


def test_fetch_tree_builds_grouped_tree_from_rich_nav():
    strat = AiGenericStrategy(Detection(platform="ai-generic", confidence="low"))
    with patch("strategies.ai_generic.requests.get",
               side_effect=_mock({ENTRY: make_response(200, RICH_HTML)})):
        tree = strat.fetch_tree(ENTRY, {})

    # Flatten all hrefs in the tree.
    hrefs = []
    def walk(ns):
        for n in ns:
            if n.href:
                hrefs.append(n.href)
            walk(n.items)
    walk(tree)

    # Same-domain doc pages are captured; external/marketing/assets/anchors dropped.
    assert "/docs/auth/login" in hrefs
    assert "/docs/api/users" in hrefs
    assert "/docs/guide/quickstart" in hrefs
    assert not any("twitter" in h for h in hrefs)
    assert "/pricing" not in hrefs
    assert not any(h.endswith(".png") for h in hrefs)
    # The "#section" anchor dedups onto /docs/api/orders (no duplicate).
    assert hrefs.count("/docs/api/orders") == 1

    # Grouped into categories (auth, api, guide) plus top-level intro.
    categories = [n.title for n in tree if not n.href and n.items]
    assert "auth" in categories and "api" in categories


def test_fetch_tree_titles_use_anchor_text():
    strat = AiGenericStrategy(Detection(platform="ai-generic", confidence="low"))
    with patch("strategies.ai_generic.requests.get",
               side_effect=_mock({ENTRY: make_response(200, RICH_HTML)})):
        tree = strat.fetch_tree(ENTRY, {})
    titles = {}
    def walk(ns):
        for n in ns:
            if n.href:
                titles[n.href] = n.title
            walk(n.items)
    walk(tree)
    assert titles["/docs/auth/login"] == "Login"
    assert titles["/docs/api/users"] == "Users API"


def test_fetch_tree_thin_nav_crawls_one_level_deeper():
    # Entry has only one link; the child page holds the rest.
    thin_entry = '<html><body><a href="/docs/section">Section</a></body></html>'
    child = """<html><body>
      <a href="/docs/section/a">A</a>
      <a href="/docs/section/b">B</a>
    </body></html>"""
    url_map = {
        "https://example.com/docs": make_response(200, thin_entry),
        "https://example.com/docs/section": make_response(200, child),
    }
    strat = AiGenericStrategy(Detection(platform="ai-generic", confidence="low"))
    with patch("strategies.ai_generic.requests.get", side_effect=_mock(url_map)):
        tree = strat.fetch_tree(ENTRY, {})
    hrefs = []
    def walk(ns):
        for n in ns:
            if n.href:
                hrefs.append(n.href)
            walk(n.items)
    walk(tree)
    assert "/docs/section/a" in hrefs
    assert "/docs/section/b" in hrefs


def test_fetch_page_extracts_main_and_flags_ai():
    html = """<html><head><title>Users API</title></head><body>
      <nav>NAV JUNK menu links</nav>
      <main><h1>Users</h1><p>GET /users returns a list.</p></main>
      <footer>FOOTER JUNK</footer>
    </body></html>"""
    strat = AiGenericStrategy(Detection(platform="ai-generic", confidence="low"))
    with patch("strategies.ai_generic.requests.get",
               side_effect=_mock({"https://example.com/docs/api/users": make_response(200, html)})):
        content = strat.fetch_page("https://example.com/docs/api/users", {})
    assert content.needs_ai is True
    assert content.title == "Users API"
    assert "GET /users" in content.raw_text
    assert "NAV JUNK" not in content.raw_text
    assert "FOOTER JUNK" not in content.raw_text


def test_main_text_drops_chrome_without_main():
    html = """<html><body>
      <header>HEADER</header><nav>NAV</nav><aside>ASIDE</aside>
      <div><p>Real content here.</p></div>
      <footer>FOOTER</footer>
    </body></html>"""
    text = _main_text(BeautifulSoup(html, "html.parser"))
    assert "Real content here." in text
    assert "HEADER" not in text and "NAV" not in text and "FOOTER" not in text


def test_group_tree_keeps_shallow_pages_top_level():
    nodes = [
        DocNode(title="Intro", href="/docs/intro"),
        DocNode(title="Login", href="/docs/auth/login"),
    ]
    tree = _group_tree(nodes, "/docs")
    top_titles = [n.title for n in tree]
    assert "Intro" in top_titles           # shallow page stays top-level
    assert "auth" in top_titles            # nested page grouped under category
