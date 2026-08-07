# API Docs — Plan 1: Python Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a standalone Python CLI tool (`tools/ApiDocFetcher/fetcher.py`) that detects API doc platform types, fetches document trees, and extracts OpenAPI specs to Markdown files.

**Architecture:** Strategy pattern — `detector.py` identifies the platform, dispatches to the matching strategy (openapi-direct / mintlify-next / swagger-ui / redoc / docusaurus / ai-generic), and `converter.py` transforms OpenAPI dicts to Markdown. All output is line-delimited JSON to stdout so Rust can parse it line-by-line.

**Tech Stack:** Python 3.9+, curl_cffi, pyyaml, beautifulsoup4, pytest, unittest.mock

---

## File Map

| File | Action | Responsibility |
|------|--------|---------------|
| `tools/ApiDocFetcher/requirements.txt` | Create | Dependencies |
| `tools/ApiDocFetcher/strategies/__init__.py` | Create | `DocNode`, `PageContent`, `Detection`, `KeepOptions` types + `Strategy` protocol |
| `tools/ApiDocFetcher/converter.py` | Create | `openapi_to_markdown(spec, keep) -> str` |
| `tools/ApiDocFetcher/detector.py` | Create | `detect(url, cookies) -> Detection` |
| `tools/ApiDocFetcher/strategies/openapi_direct.py` | Create | Try `/openapi.json` etc, return tree + page content |
| `tools/ApiDocFetcher/strategies/mintlify_next.py` | Create | Parse `__next_f` chunks for tree + OpenAPI YAML |
| `tools/ApiDocFetcher/strategies/swagger_ui.py` | Create | Detect SwaggerUI spec URL, delegate to openapi_direct |
| `tools/ApiDocFetcher/strategies/redoc.py` | Create | Detect Redoc spec URL, delegate to openapi_direct |
| `tools/ApiDocFetcher/strategies/docusaurus.py` | Create | Fetch `sidebar.json`, scrape MD pages |
| `tools/ApiDocFetcher/strategies/ai_generic.py` | Create | BeautifulSoup text extraction, `needs_ai=True` |
| `tools/ApiDocFetcher/fetcher.py` | Create | CLI: `detect` / `tree` / `extract` subcommands |
| `tools/ApiDocFetcher/tests/conftest.py` | Create | Shared fixtures |
| `tools/ApiDocFetcher/tests/test_converter.py` | Create | Converter unit tests |
| `tools/ApiDocFetcher/tests/test_detector.py` | Create | Detector unit tests (mocked HTTP) |
| `tools/ApiDocFetcher/tests/test_strategies.py` | Create | Per-strategy unit tests (mocked HTTP) |
| `tools/SwiftDocFetcher/` | Keep | probe.js stays for reference; Python files go in ApiDocFetcher |

---

### Task 1: Directory scaffold + requirements

**Files:**
- Create: `tools/ApiDocFetcher/requirements.txt`
- Create: `tools/ApiDocFetcher/strategies/__init__.py`
- Create: `tools/ApiDocFetcher/tests/__init__.py`
- Create: `tools/ApiDocFetcher/tests/conftest.py`

- [ ] **Step 1: Create requirements.txt**

```
curl_cffi>=0.7.0
pyyaml>=6.0
beautifulsoup4>=4.12
pytest>=8.0
```

- [ ] **Step 2: Create strategies/__init__.py with shared types**

```python
# tools/ApiDocFetcher/strategies/__init__.py
from dataclasses import dataclass, field
from typing import Optional, Protocol


@dataclass
class DocNode:
    title: str
    href: str
    items: list["DocNode"] = field(default_factory=list)

    def to_dict(self) -> dict:
        return {
            "title": self.title,
            "href": self.href,
            "items": [c.to_dict() for c in self.items],
        }


@dataclass
class PageContent:
    title: str
    openapi_spec: Optional[dict] = None  # parsed OpenAPI dict
    raw_text: Optional[str] = None       # for ai_generic fallback
    needs_ai: bool = False
    platform: str = ""


@dataclass
class Detection:
    platform: str  # openapi-direct | mintlify-next | swagger-ui | redoc | docusaurus | ai-generic
    confidence: str  # high | medium | low
    spec_url: Optional[str] = None


@dataclass
class KeepOptions:
    description: bool = True
    parameters: bool = True
    request_schema: bool = True
    response_schema: bool = True
    code_samples: bool = True

    @classmethod
    def from_dict(cls, d: dict) -> "KeepOptions":
        return cls(**{k: v for k, v in d.items() if k in cls.__dataclass_fields__})


class Strategy(Protocol):
    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]: ...
    def fetch_page(self, url: str, cookies: dict) -> PageContent: ...


def get_strategy(detection: Detection) -> Strategy:
    """Return the matching strategy instance for a Detection."""
    from strategies.openapi_direct import OpenApiDirectStrategy
    from strategies.mintlify_next import MintlifyNextStrategy
    from strategies.swagger_ui import SwaggerUiStrategy
    from strategies.redoc import RedocStrategy
    from strategies.docusaurus import DocusaurusStrategy
    from strategies.ai_generic import AiGenericStrategy

    mapping = {
        "openapi-direct": OpenApiDirectStrategy,
        "mintlify-next": MintlifyNextStrategy,
        "swagger-ui": SwaggerUiStrategy,
        "redoc": RedocStrategy,
        "docusaurus": DocusaurusStrategy,
        "ai-generic": AiGenericStrategy,
    }
    cls = mapping.get(detection.platform, AiGenericStrategy)
    return cls(detection)
```

- [ ] **Step 3: Create tests/conftest.py**

```python
# tools/ApiDocFetcher/tests/conftest.py
from unittest.mock import MagicMock


def make_response(status: int = 200, text: str = "") -> MagicMock:
    """Factory for mocked curl_cffi responses."""
    r = MagicMock()
    r.status_code = status
    r.text = text
    return r
```

- [ ] **Step 4: Create tests/__init__.py** (empty file)

```
(empty)
```

- [ ] **Step 5: Install dependencies**

```bash
cd tools/ApiDocFetcher
pip install -r requirements.txt
```

Expected: packages installed without errors.

- [ ] **Step 6: Commit**

```bash
git add tools/ApiDocFetcher/
git commit -m "feat(api-docs): scaffold Python backend structure"
```

---

### Task 2: converter.py — OpenAPI → Markdown

**Files:**
- Create: `tools/ApiDocFetcher/converter.py`
- Create: `tools/ApiDocFetcher/tests/test_converter.py`

- [ ] **Step 1: Write failing tests**

```python
# tools/ApiDocFetcher/tests/test_converter.py
import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from converter import openapi_to_markdown
from strategies import KeepOptions


SIMPLE_SPEC = {
    "openapi": "3.0.0",
    "info": {"title": "Test API", "version": "1.0.0", "description": "A test API."},
    "servers": [{"url": "https://api.example.com"}],
    "paths": {
        "/users": {
            "get": {
                "summary": "List users",
                "parameters": [
                    {
                        "name": "limit",
                        "in": "query",
                        "required": False,
                        "schema": {"type": "integer"},
                        "description": "Maximum results",
                    }
                ],
                "requestBody": {
                    "content": {
                        "application/json": {
                            "schema": {"type": "object", "properties": {"q": {"type": "string"}}}
                        }
                    }
                },
                "responses": {
                    "200": {"description": "Success"},
                    "400": {"description": "Bad Request"},
                },
            }
        }
    },
}


def test_header_contains_title_and_version():
    md = openapi_to_markdown(SIMPLE_SPEC, KeepOptions())
    assert "# Test API — 1.0.0" in md


def test_base_url_included():
    md = openapi_to_markdown(SIMPLE_SPEC, KeepOptions())
    assert "https://api.example.com" in md


def test_endpoint_heading():
    md = openapi_to_markdown(SIMPLE_SPEC, KeepOptions())
    assert "## GET /users" in md


def test_parameters_table():
    md = openapi_to_markdown(SIMPLE_SPEC, KeepOptions())
    assert "| `limit`" in md
    assert "Maximum results" in md


def test_response_table():
    md = openapi_to_markdown(SIMPLE_SPEC, KeepOptions())
    assert "| 200 |" in md
    assert "| 400 |" in md


def test_keep_parameters_false_omits_params():
    keep = KeepOptions(parameters=False)
    md = openapi_to_markdown(SIMPLE_SPEC, keep)
    assert "### Parameters" not in md


def test_keep_response_schema_false_omits_responses():
    keep = KeepOptions(response_schema=False)
    md = openapi_to_markdown(SIMPLE_SPEC, keep)
    assert "### Responses" not in md


def test_empty_paths_produces_valid_header():
    spec = {"openapi": "3.0.0", "info": {"title": "Empty", "version": "0.1"}, "paths": {}}
    md = openapi_to_markdown(spec, KeepOptions())
    assert "# Empty — 0.1" in md
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd tools/ApiDocFetcher
pytest tests/test_converter.py -v
```

Expected: `ModuleNotFoundError: No module named 'converter'`

- [ ] **Step 3: Implement converter.py**

```python
# tools/ApiDocFetcher/converter.py
import json
from strategies import KeepOptions


def openapi_to_markdown(spec: dict, keep: KeepOptions) -> str:
    """Convert a parsed OpenAPI 3.x spec dict to a Markdown string."""
    lines: list[str] = []
    info = spec.get("info", {})
    title = info.get("title", "API Reference")
    version = info.get("version", "")
    description = info.get("description", "")
    servers = spec.get("servers", [])

    lines += [f"# {title} — {version}", ""]

    if description and keep.description:
        first_line = description.strip().split("\n")[0]
        lines += [f"> {first_line}", ""]

    if servers:
        lines += [f"**Base URL**: `{servers[0].get('url', '')}`", ""]

    lines += ["---", ""]

    paths = spec.get("paths", {})
    for path, path_item in paths.items():
        if not isinstance(path_item, dict):
            continue
        for method in ("get", "post", "put", "patch", "delete", "head", "options"):
            op = path_item.get(method)
            if not op:
                continue

            lines += [f"## {method.upper()} {path}", ""]

            summary = op.get("summary", "")
            if summary:
                lines += [f"**Summary**: {summary}", ""]

            op_desc = op.get("description", "")
            if keep.description and op_desc:
                lines += [op_desc.strip(), ""]

            # Parameters
            if keep.parameters:
                params = op.get("parameters", [])
                if params:
                    lines += [
                        "### Parameters",
                        "",
                        "| Name | In | Type | Required | Description |",
                        "|------|----|------|----------|-------------|",
                    ]
                    for p in params:
                        name = p.get("name", "")
                        loc = p.get("in", "")
                        required = "✓" if p.get("required") else ""
                        desc = p.get("description", "").replace("\n", " ")
                        ptype = p.get("schema", {}).get("type", "")
                        lines.append(f"| `{name}` | {loc} | {ptype} | {required} | {desc} |")
                    lines.append("")

            # Request body
            if keep.request_schema:
                req_body = op.get("requestBody", {})
                content = req_body.get("content", {})
                for media_type, media_obj in content.items():
                    schema = media_obj.get("schema")
                    if schema:
                        lines += [
                            "### Request Body",
                            "",
                            f"Content-Type: `{media_type}`",
                            "",
                            "```json",
                            json.dumps(schema, indent=2),
                            "```",
                            "",
                        ]
                    break  # only first media type

            # Responses
            if keep.response_schema:
                responses = op.get("responses", {})
                if responses:
                    lines += [
                        "### Responses",
                        "",
                        "| Code | Description |",
                        "|------|-------------|",
                    ]
                    for code, resp in responses.items():
                        desc = resp.get("description", "") if isinstance(resp, dict) else ""
                        lines.append(f"| {code} | {desc} |")
                    lines.append("")

            # Code samples (x-codeSamples extension)
            if keep.code_samples:
                for sample in op.get("x-codeSamples", []):
                    lang = sample.get("lang", "bash")
                    src = sample.get("source", "")
                    lines += [f"### Example ({lang})", "", f"```{lang.lower()}", src, "```", ""]

            lines += ["---", ""]

    return "\n".join(lines)
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd tools/ApiDocFetcher
pytest tests/test_converter.py -v
```

Expected: `8 passed`

- [ ] **Step 5: Commit**

```bash
git add tools/ApiDocFetcher/converter.py tools/ApiDocFetcher/tests/test_converter.py
git commit -m "feat(api-docs): add OpenAPI→Markdown converter"
```

---

### Task 3: detector.py — platform detection

**Files:**
- Create: `tools/ApiDocFetcher/detector.py`
- Create: `tools/ApiDocFetcher/tests/test_detector.py`

- [ ] **Step 1: Write failing tests**

```python
# tools/ApiDocFetcher/tests/test_detector.py
import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from unittest.mock import patch, call
from tests.conftest import make_response
from detector import detect


def _mock_get_factory(url_map: dict):
    """Return side_effect function that maps URLs to mock responses."""
    def side_effect(url, **kwargs):
        return url_map.get(url, make_response(404, ""))
    return side_effect


def test_detect_openapi_json():
    url_map = {
        "https://example.com/openapi.json": make_response(200, "openapi: 3.0.0\npaths: {}")
    }
    with patch("detector.requests.get", side_effect=_mock_get_factory(url_map)):
        result = detect("https://example.com/docs", {})
    assert result.platform == "openapi-direct"
    assert result.confidence == "high"
    assert result.spec_url == "https://example.com/openapi.json"


def test_detect_swagger_json():
    url_map = {
        "https://example.com/swagger.json": make_response(200, '{"swagger":"2.0","paths":{}}')
    }
    with patch("detector.requests.get", side_effect=_mock_get_factory(url_map)):
        result = detect("https://example.com/docs", {})
    assert result.platform == "openapi-direct"
    assert result.spec_url == "https://example.com/swagger.json"


def test_detect_mintlify_next():
    html = '<html><script>self.__next_f.push([1,"openapi: 3.0.0\\npaths: {}"])</script></html>'
    url_map = {"https://docs.example.com/docs": make_response(200, html)}
    with patch("detector.requests.get", side_effect=_mock_get_factory(url_map)):
        result = detect("https://docs.example.com/docs", {})
    assert result.platform == "mintlify-next"
    assert result.confidence == "high"


def test_detect_swagger_ui():
    html = '<html><div id="swagger-ui"></div><script>SwaggerUIBundle({url:"/spec.json"})</script></html>'
    url_map = {"https://example.com/docs": make_response(200, html)}
    with patch("detector.requests.get", side_effect=_mock_get_factory(url_map)):
        result = detect("https://example.com/docs", {})
    assert result.platform == "swagger-ui"
    assert result.confidence == "high"


def test_detect_redoc():
    html = '<html><redoc spec-url="/openapi.yaml"></redoc></html>'
    url_map = {"https://example.com/docs": make_response(200, html)}
    with patch("detector.requests.get", side_effect=_mock_get_factory(url_map)):
        result = detect("https://example.com/docs", {})
    assert result.platform == "redoc"
    assert result.spec_url == "https://example.com/openapi.yaml"


def test_detect_docusaurus():
    html = '<html><script>__docusaurus</script></html>'
    url_map = {"https://example.com/docs": make_response(200, html)}
    with patch("detector.requests.get", side_effect=_mock_get_factory(url_map)):
        result = detect("https://example.com/docs", {})
    assert result.platform == "docusaurus"


def test_detect_ai_generic_fallback():
    html = "<html><body>Some random API docs</body></html>"
    url_map = {"https://example.com/docs": make_response(200, html)}
    with patch("detector.requests.get", side_effect=_mock_get_factory(url_map)):
        result = detect("https://example.com/docs", {})
    assert result.platform == "ai-generic"
    assert result.confidence == "low"


def test_detect_passes_cookies():
    html = "<html><body>plain</body></html>"
    with patch("detector.requests.get", return_value=make_response(200, html)) as mock_get:
        detect("https://example.com/docs", {"session": "abc"})
    _, kwargs = mock_get.call_args
    assert kwargs.get("cookies") == {"session": "abc"}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd tools/ApiDocFetcher
pytest tests/test_detector.py -v
```

Expected: `ModuleNotFoundError: No module named 'detector'`

- [ ] **Step 3: Implement detector.py**

```python
# tools/ApiDocFetcher/detector.py
import re
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

    if "__next_f" in html and "openapi:" in html:
        return Detection(platform="mintlify-next", confidence="high")

    if "swagger-ui" in html.lower() or "SwaggerUIBundle" in html:
        spec_url = _extract_swagger_spec_url(html, base)
        return Detection(platform="swagger-ui", confidence="high", spec_url=spec_url)

    if re.search(r"<redoc[\s>]", html, re.IGNORECASE) or "ReDoc.init" in html:
        spec_url = _extract_redoc_spec_url(html, base)
        return Detection(platform="redoc", confidence="high", spec_url=spec_url)

    if "__docusaurus" in html or "docusaurus" in html:
        return Detection(platform="docusaurus", confidence="medium")

    return Detection(platform="ai-generic", confidence="low")


def _looks_like_openapi(text: str) -> bool:
    t = text.strip()[:200].lower()
    return "openapi" in t or "swagger" in t


def _extract_swagger_spec_url(html: str, base: str) -> str | None:
    for pat in [
        r'url\s*:\s*["\']([^"\']+\.(?:json|yaml))["\']',
        r'SwaggerUIBundle\b[^)]*?url\s*:\s*["\']([^"\']+)["\']',
    ]:
        m = re.search(pat, html)
        if m:
            u = m.group(1)
            return u if u.startswith("http") else base + u
    return None


def _extract_redoc_spec_url(html: str, base: str) -> str | None:
    m = re.search(r'<redoc[^>]+spec-url=["\']([^"\']+)["\']', html, re.IGNORECASE)
    if m:
        u = m.group(1)
        return u if u.startswith("http") else base + u
    m = re.search(r'ReDoc\.init\(["\']([^"\']+)["\']', html)
    if m:
        u = m.group(1)
        return u if u.startswith("http") else base + u
    return None
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd tools/ApiDocFetcher
pytest tests/test_detector.py -v
```

Expected: `8 passed`

- [ ] **Step 5: Commit**

```bash
git add tools/ApiDocFetcher/detector.py tools/ApiDocFetcher/tests/test_detector.py
git commit -m "feat(api-docs): add platform detector"
```

---

### Task 4: strategies/openapi_direct.py

**Files:**
- Create: `tools/ApiDocFetcher/strategies/openapi_direct.py`
- Modify: `tools/ApiDocFetcher/tests/test_strategies.py` (create file)

- [ ] **Step 1: Write failing tests**

```python
# tools/ApiDocFetcher/tests/test_strategies.py
import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import json
import yaml
from unittest.mock import patch
from tests.conftest import make_response
from strategies import Detection, KeepOptions
from strategies.openapi_direct import OpenApiDirectStrategy


SAMPLE_OPENAPI_YAML = """
openapi: 3.0.0
info:
  title: Sample API
  version: 1.0.0
tags:
  - name: Users
paths:
  /users:
    get:
      tags: [Users]
      summary: List users
      responses:
        '200':
          description: OK
"""


def test_openapi_direct_fetch_tree_groups_by_tag():
    detection = Detection(platform="openapi-direct", confidence="high",
                          spec_url="https://example.com/openapi.yaml")
    with patch("strategies.openapi_direct.requests.get",
               return_value=make_response(200, SAMPLE_OPENAPI_YAML)):
        strategy = OpenApiDirectStrategy(detection)
        tree = strategy.fetch_tree("https://example.com/docs", {})
    assert len(tree) == 1
    assert tree[0].title == "Users"
    assert any(n.href == "/users" for n in tree[0].items)


def test_openapi_direct_fetch_page_returns_spec():
    detection = Detection(platform="openapi-direct", confidence="high",
                          spec_url="https://example.com/openapi.yaml")
    with patch("strategies.openapi_direct.requests.get",
               return_value=make_response(200, SAMPLE_OPENAPI_YAML)):
        strategy = OpenApiDirectStrategy(detection)
        content = strategy.fetch_page("https://example.com/docs", {})
    assert content.openapi_spec is not None
    assert content.openapi_spec["info"]["title"] == "Sample API"
    assert not content.needs_ai


def test_openapi_direct_json_spec():
    spec_json = json.dumps({"openapi": "3.0.0", "info": {"title": "JSON API", "version": "2.0"},
                             "paths": {}})
    detection = Detection(platform="openapi-direct", confidence="high",
                          spec_url="https://example.com/openapi.json")
    with patch("strategies.openapi_direct.requests.get",
               return_value=make_response(200, spec_json)):
        strategy = OpenApiDirectStrategy(detection)
        content = strategy.fetch_page("https://example.com/docs", {})
    assert content.openapi_spec["info"]["title"] == "JSON API"
```

- [ ] **Step 2: Run to verify failure**

```bash
cd tools/ApiDocFetcher
pytest tests/test_strategies.py::test_openapi_direct_fetch_tree_groups_by_tag -v
```

Expected: `ModuleNotFoundError: No module named 'strategies.openapi_direct'`

- [ ] **Step 3: Implement strategies/openapi_direct.py**

```python
# tools/ApiDocFetcher/strategies/openapi_direct.py
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
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd tools/ApiDocFetcher
pytest tests/test_strategies.py -k "openapi_direct" -v
```

Expected: `3 passed`

- [ ] **Step 5: Commit**

```bash
git add tools/ApiDocFetcher/strategies/openapi_direct.py tools/ApiDocFetcher/tests/test_strategies.py
git commit -m "feat(api-docs): add openapi-direct strategy"
```

---

### Task 5: strategies/mintlify_next.py

**Files:**
- Create: `tools/ApiDocFetcher/strategies/mintlify_next.py`
- Modify: `tools/ApiDocFetcher/tests/test_strategies.py`

- [ ] **Step 1: Add tests (append to test_strategies.py)**

```python
# Append to tools/ApiDocFetcher/tests/test_strategies.py

from strategies.mintlify_next import MintlifyNextStrategy

MINTLIFY_TREE_HTML = """
<html><body>
<script>self.__next_f.push([1,"c:[\\\"$\\\",\\\"$L15\\\",null,{\\\"items\\\":[{\\\"title\\\":\\\"Getting Started\\\",\\\"href\\\":\\\"/docs/start\\\",\\\"items\\\":[{\\\"title\\\":\\\"Intro\\\",\\\"href\\\":\\\"/docs/start/intro\\\",\\\"items\\\":[]}]}]}]"])</script>
</body></html>
"""

MINTLIFY_SPEC_HTML = """
<html><body>
<script>self.__next_f.push([1,"openapi: 3.0.0\\ninfo:\\n  title: SWIFT API\\n  version: 1.0.0\\npaths: {}"])</script>
</body></html>
"""


def test_mintlify_fetch_tree_parses_nav():
    detection = Detection(platform="mintlify-next", confidence="high")
    with patch("strategies.mintlify_next.requests.get",
               return_value=make_response(200, MINTLIFY_TREE_HTML)):
        strategy = MintlifyNextStrategy(detection)
        tree = strategy.fetch_tree("https://docs.example.com/docs", {})
    assert len(tree) >= 1
    assert tree[0].title == "Getting Started"
    assert tree[0].href == "/docs/start"


def test_mintlify_fetch_page_extracts_openapi():
    detection = Detection(platform="mintlify-next", confidence="high")
    with patch("strategies.mintlify_next.requests.get",
               return_value=make_response(200, MINTLIFY_SPEC_HTML)):
        strategy = MintlifyNextStrategy(detection)
        content = strategy.fetch_page("https://docs.example.com/docs/api", {})
    assert content.openapi_spec is not None
    assert content.openapi_spec["info"]["title"] == "SWIFT API"
    assert not content.needs_ai
```

- [ ] **Step 2: Run to verify failure**

```bash
cd tools/ApiDocFetcher
pytest tests/test_strategies.py -k "mintlify" -v
```

Expected: `ModuleNotFoundError: No module named 'strategies.mintlify_next'`

- [ ] **Step 3: Implement strategies/mintlify_next.py**

```python
# tools/ApiDocFetcher/strategies/mintlify_next.py
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
            # Find the first JSON array with nav items
            m = re.search(r'\[(\{"title".*)\]', content, re.DOTALL)
            if not m:
                continue
            try:
                raw = json.loads("[" + m.group(1) + "]")
                return [_dict_to_node(item) for item in raw]
            except (json.JSONDecodeError, KeyError):
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
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd tools/ApiDocFetcher
pytest tests/test_strategies.py -k "mintlify" -v
```

Expected: `2 passed`

- [ ] **Step 5: Commit**

```bash
git add tools/ApiDocFetcher/strategies/mintlify_next.py tools/ApiDocFetcher/tests/test_strategies.py
git commit -m "feat(api-docs): add mintlify-next strategy (SWIFT validated)"
```

---

### Task 6: strategies/swagger_ui.py + redoc.py

**Files:**
- Create: `tools/ApiDocFetcher/strategies/swagger_ui.py`
- Create: `tools/ApiDocFetcher/strategies/redoc.py`
- Modify: `tools/ApiDocFetcher/tests/test_strategies.py`

- [ ] **Step 1: Add tests (append to test_strategies.py)**

```python
# Append to tools/ApiDocFetcher/tests/test_strategies.py

from strategies.swagger_ui import SwaggerUiStrategy
from strategies.redoc import RedocStrategy


def test_swagger_ui_fetch_page_uses_spec_url():
    detection = Detection(platform="swagger-ui", confidence="high",
                          spec_url="https://example.com/spec.json")
    spec_json = json.dumps({"openapi": "3.0.0", "info": {"title": "Swagger API", "version": "1"},
                             "paths": {}})
    with patch("strategies.swagger_ui.requests.get",
               return_value=make_response(200, spec_json)):
        strategy = SwaggerUiStrategy(detection)
        content = strategy.fetch_page("https://example.com/docs", {})
    assert content.openapi_spec["info"]["title"] == "Swagger API"


def test_swagger_ui_fetch_tree_returns_tag_groups():
    spec_yaml = """
openapi: 3.0.0
info:
  title: Swagger API
  version: "1"
tags:
  - name: Pets
paths:
  /pets:
    get:
      tags: [Pets]
      summary: List pets
      responses:
        '200':
          description: OK
"""
    detection = Detection(platform="swagger-ui", confidence="high",
                          spec_url="https://example.com/spec.yaml")
    with patch("strategies.swagger_ui.requests.get",
               return_value=make_response(200, spec_yaml)):
        strategy = SwaggerUiStrategy(detection)
        tree = strategy.fetch_tree("https://example.com/docs", {})
    assert tree[0].title == "Pets"


def test_redoc_fetch_page_uses_spec_url():
    detection = Detection(platform="redoc", confidence="high",
                          spec_url="https://example.com/openapi.yaml")
    spec_yaml = "openapi: 3.0.0\ninfo:\n  title: Redoc API\n  version: '1'\npaths: {}"
    with patch("strategies.redoc.requests.get",
               return_value=make_response(200, spec_yaml)):
        strategy = RedocStrategy(detection)
        content = strategy.fetch_page("https://example.com/docs", {})
    assert content.openapi_spec["info"]["title"] == "Redoc API"
```

- [ ] **Step 2: Run to verify failure**

```bash
cd tools/ApiDocFetcher
pytest tests/test_strategies.py -k "swagger_ui or redoc" -v
```

Expected: `ModuleNotFoundError`

- [ ] **Step 3: Implement strategies/swagger_ui.py**

Both swagger-ui and redoc delegate to `OpenApiDirectStrategy` once the spec URL is known — DRY.

```python
# tools/ApiDocFetcher/strategies/swagger_ui.py
from strategies import Detection, DocNode, PageContent
from strategies.openapi_direct import OpenApiDirectStrategy
from curl_cffi import requests


class SwaggerUiStrategy:
    """Swagger UI sites expose an OpenAPI spec URL — delegate to OpenApiDirectStrategy."""

    def __init__(self, detection: Detection):
        self._inner = OpenApiDirectStrategy(detection)

    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]:
        return self._inner.fetch_tree(url, cookies)

    def fetch_page(self, url: str, cookies: dict) -> PageContent:
        return self._inner.fetch_page(url, cookies)
```

- [ ] **Step 4: Implement strategies/redoc.py**

```python
# tools/ApiDocFetcher/strategies/redoc.py
from strategies import Detection, DocNode, PageContent
from strategies.openapi_direct import OpenApiDirectStrategy
from curl_cffi import requests


class RedocStrategy:
    """Redoc sites expose an OpenAPI spec URL — delegate to OpenApiDirectStrategy."""

    def __init__(self, detection: Detection):
        self._inner = OpenApiDirectStrategy(detection)

    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]:
        return self._inner.fetch_tree(url, cookies)

    def fetch_page(self, url: str, cookies: dict) -> PageContent:
        return self._inner.fetch_page(url, cookies)
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cd tools/ApiDocFetcher
pytest tests/test_strategies.py -k "swagger_ui or redoc" -v
```

Expected: `3 passed`

- [ ] **Step 6: Commit**

```bash
git add tools/ApiDocFetcher/strategies/swagger_ui.py tools/ApiDocFetcher/strategies/redoc.py tools/ApiDocFetcher/tests/test_strategies.py
git commit -m "feat(api-docs): add swagger-ui and redoc strategies"
```

---

### Task 7: strategies/docusaurus.py + ai_generic.py

**Files:**
- Create: `tools/ApiDocFetcher/strategies/docusaurus.py`
- Create: `tools/ApiDocFetcher/strategies/ai_generic.py`
- Modify: `tools/ApiDocFetcher/tests/test_strategies.py`

- [ ] **Step 1: Add tests**

```python
# Append to tools/ApiDocFetcher/tests/test_strategies.py

from strategies.docusaurus import DocusaurusStrategy
from strategies.ai_generic import AiGenericStrategy


DOCUSAURUS_SIDEBAR = json.dumps({
    "docs": [
        {"type": "category", "label": "Getting Started",
         "items": [{"type": "doc", "id": "intro", "label": "Introduction"}]},
    ]
})


def test_docusaurus_fetch_tree_from_sidebar_json():
    detection = Detection(platform="docusaurus", confidence="medium")
    url_map = {
        "https://docs.example.com/docs/sidebar.json": make_response(200, DOCUSAURUS_SIDEBAR),
    }
    with patch("strategies.docusaurus.requests.get",
               side_effect=lambda url, **kw: url_map.get(url, make_response(404, ""))):
        strategy = DocusaurusStrategy(detection)
        tree = strategy.fetch_tree("https://docs.example.com/docs", {})
    assert tree[0].title == "Getting Started"
    assert tree[0].items[0].title == "Introduction"


def test_ai_generic_returns_raw_text_needs_ai():
    html = "<html><body><h1>API Docs</h1><p>POST /users creates a user.</p></body></html>"
    detection = Detection(platform="ai-generic", confidence="low")
    with patch("strategies.ai_generic.requests.get",
               return_value=make_response(200, html)):
        strategy = AiGenericStrategy(detection)
        content = strategy.fetch_page("https://example.com/docs/api", {})
    assert content.needs_ai is True
    assert "POST /users" in (content.raw_text or "")
    assert content.openapi_spec is None


def test_ai_generic_fetch_tree_returns_empty():
    detection = Detection(platform="ai-generic", confidence="low")
    with patch("strategies.ai_generic.requests.get",
               return_value=make_response(200, "<html><body>docs</body></html>")):
        strategy = AiGenericStrategy(detection)
        tree = strategy.fetch_tree("https://example.com/docs", {})
    assert tree == []
```

- [ ] **Step 2: Run to verify failure**

```bash
cd tools/ApiDocFetcher
pytest tests/test_strategies.py -k "docusaurus or ai_generic" -v
```

Expected: `ModuleNotFoundError`

- [ ] **Step 3: Implement strategies/docusaurus.py**

```python
# tools/ApiDocFetcher/strategies/docusaurus.py
import json
from curl_cffi import requests
from strategies import Detection, DocNode, PageContent
from bs4 import BeautifulSoup

SIDEBAR_PATHS = ["/docs/sidebar.json", "/_docusaurus/sidebar.json", "/sidebar.json"]


class DocusaurusStrategy:
    def __init__(self, detection: Detection):
        self._detection = detection

    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]:
        base = url.rstrip("/").split("/docs")[0]
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
```

- [ ] **Step 4: Implement strategies/ai_generic.py**

```python
# tools/ApiDocFetcher/strategies/ai_generic.py
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
```

- [ ] **Step 5: Run all strategy tests**

```bash
cd tools/ApiDocFetcher
pytest tests/test_strategies.py -v
```

Expected: all tests pass (≥11 passed)

- [ ] **Step 6: Commit**

```bash
git add tools/ApiDocFetcher/strategies/docusaurus.py tools/ApiDocFetcher/strategies/ai_generic.py tools/ApiDocFetcher/tests/test_strategies.py
git commit -m "feat(api-docs): add docusaurus and ai-generic strategies"
```

---

### Task 8: fetcher.py — CLI entry point

**Files:**
- Create: `tools/ApiDocFetcher/fetcher.py`

- [ ] **Step 1: Create fetcher.py**

```python
#!/usr/bin/env python3
# tools/ApiDocFetcher/fetcher.py
"""
ApiDocFetcher CLI.

Commands:
  detect  --url URL [--cookies COOKIE_STR]
  tree    --url URL [--cookies COOKIE_STR]
  extract --url URL --pages JSON_ARRAY --output DIR [--merge] [--keep JSON] [--cookies COOKIE_STR]

All output is line-delimited JSON to stdout.
"""
import argparse
import json
import os
import sys

# Ensure strategies package is importable when run from any working dir
sys.path.insert(0, os.path.dirname(__file__))

from detector import detect
from strategies import get_strategy, KeepOptions
from converter import openapi_to_markdown


def _parse_cookies(cookie_str: str) -> dict:
    if not cookie_str:
        return {}
    result = {}
    for part in cookie_str.split(";"):
        if "=" in part:
            k, v = part.strip().split("=", 1)
            result[k.strip()] = v.strip()
    return result


def _emit(obj: dict) -> None:
    print(json.dumps(obj, ensure_ascii=False), flush=True)


def _node_to_dict(node) -> dict:
    return {"title": node.title, "href": node.href,
            "items": [_node_to_dict(c) for c in node.items]}


def cmd_detect(args) -> None:
    cookies = _parse_cookies(args.cookies)
    result = detect(args.url, cookies)
    _emit({"type": "detected", "platform": result.platform,
           "confidence": result.confidence, "spec_url": result.spec_url})


def cmd_tree(args) -> None:
    cookies = _parse_cookies(args.cookies)
    detection = detect(args.url, cookies)
    _emit({"type": "detected", "platform": detection.platform,
           "confidence": detection.confidence})
    strategy = get_strategy(detection)
    nodes = strategy.fetch_tree(args.url, cookies)
    _emit({"type": "tree", "data": [_node_to_dict(n) for n in nodes]})


def cmd_extract(args) -> None:
    cookies = _parse_cookies(args.cookies)
    pages: list[dict] = json.loads(args.pages)
    keep = KeepOptions.from_dict(json.loads(args.keep)) if args.keep else KeepOptions()

    detection = detect(args.url, cookies)
    _emit({"type": "detected", "platform": detection.platform,
           "confidence": detection.confidence})
    strategy = get_strategy(detection)

    total = len(pages)
    markdowns: list[tuple[dict, str]] = []

    for i, page in enumerate(pages):
        base = args.url.rstrip("/").split("/docs")[0]
        href = page.get("href", "")
        page_url = href if href.startswith("http") else base + href

        _emit({"type": "progress", "current": i + 1, "total": total,
               "page": page.get("title", href)})
        try:
            content = strategy.fetch_page(page_url, cookies)
            if content.needs_ai:
                # Signal to Rust that AI processing is needed for this page
                _emit({"type": "needs_ai", "title": content.title,
                       "raw_text": content.raw_text or ""})
                md = f"# {content.title}\n\n> ⚠ AI processing required\n\n{content.raw_text or ''}"
            elif content.openapi_spec:
                md = openapi_to_markdown(content.openapi_spec, keep)
            else:
                md = f"# {content.title}\n\n{content.raw_text or ''}"

            size_kb = len(md.encode()) // 1024
            _emit({"type": "log", "level": "info",
                   "message": f"✓ {page.get('title', href)} ({size_kb}KB)"})
            markdowns.append((page, md))
        except Exception as exc:
            _emit({"type": "log", "level": "error",
                   "message": f"✗ Failed: {page.get('title', href)}: {exc}"})

    os.makedirs(args.output, exist_ok=True)
    files: list[str] = []

    if args.merge:
        out_path = os.path.join(args.output, "api-docs.md")
        with open(out_path, "w", encoding="utf-8") as f:
            f.write("\n\n---\n\n".join(md for _, md in markdowns))
        files.append(out_path)
    else:
        for page, md in markdowns:
            slug = page.get("href", "page").strip("/").split("/")[-1] or "page"
            out_path = os.path.join(args.output, f"{slug}.md")
            with open(out_path, "w", encoding="utf-8") as f:
                f.write(md)
            files.append(out_path)

    _emit({"type": "done", "files": files})


def main() -> None:
    parser = argparse.ArgumentParser(description="ApiDocFetcher CLI")
    sub = parser.add_subparsers(dest="cmd", required=True)

    for name in ("detect", "tree"):
        p = sub.add_parser(name)
        p.add_argument("--url", required=True)
        p.add_argument("--cookies", default="")

    p_ex = sub.add_parser("extract")
    p_ex.add_argument("--url", required=True)
    p_ex.add_argument("--pages", required=True, help="JSON array of {title, href}")
    p_ex.add_argument("--output", required=True)
    p_ex.add_argument("--merge", action="store_true")
    p_ex.add_argument("--keep", default="", help="JSON KeepOptions")
    p_ex.add_argument("--cookies", default="")

    args = parser.parse_args()
    {"detect": cmd_detect, "tree": cmd_tree, "extract": cmd_extract}[args.cmd](args)


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Smoke test the CLI**

```bash
cd tools/ApiDocFetcher
python3 fetcher.py detect --url https://httpbin.org
```

Expected output (one line of JSON):
```json
{"type": "detected", "platform": "ai-generic", "confidence": "low", "spec_url": null}
```

- [ ] **Step 3: Run full test suite**

```bash
cd tools/ApiDocFetcher
pytest tests/ -v
```

Expected: all tests pass (≥19 passed)

- [ ] **Step 4: Commit**

```bash
git add tools/ApiDocFetcher/fetcher.py
git commit -m "feat(api-docs): add fetcher.py CLI entry point"
```

---

### Task 9: Final validation

- [ ] **Step 1: Run complete test suite**

```bash
cd tools/ApiDocFetcher
pytest tests/ -v --tb=short
```

Expected: all tests pass, no warnings about missing modules.

- [ ] **Step 2: Verify CLI help**

```bash
python3 fetcher.py --help
python3 fetcher.py detect --help
python3 fetcher.py tree --help
python3 fetcher.py extract --help
```

Expected: usage text for each subcommand.

- [ ] **Step 3: Tag plan 1 complete**

```bash
git add tools/ApiDocFetcher/
git commit -m "feat(api-docs): Python backend complete — Plan 1 done"
```

---

> **Next:** Proceed to `docs/superpowers/plans/2026-06-01-api-docs-rust-bridge.md` (Plan 2).
