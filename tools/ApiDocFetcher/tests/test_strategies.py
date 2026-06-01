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


from strategies.swagger_ui import SwaggerUiStrategy
from strategies.redoc import RedocStrategy


def test_swagger_ui_fetch_page_uses_spec_url():
    detection = Detection(platform="swagger-ui", confidence="high",
                          spec_url="https://example.com/spec.json")
    spec_json = json.dumps({"openapi": "3.0.0", "info": {"title": "Swagger API", "version": "1"},
                             "paths": {}})
    with patch("strategies.openapi_direct.requests.get",
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
    with patch("strategies.openapi_direct.requests.get",
               return_value=make_response(200, spec_yaml)):
        strategy = SwaggerUiStrategy(detection)
        tree = strategy.fetch_tree("https://example.com/docs", {})
    assert tree[0].title == "Pets"


def test_redoc_fetch_page_uses_spec_url():
    detection = Detection(platform="redoc", confidence="high",
                          spec_url="https://example.com/openapi.yaml")
    spec_yaml = "openapi: 3.0.0\ninfo:\n  title: Redoc API\n  version: '1'\npaths: {}"
    with patch("strategies.openapi_direct.requests.get",
               return_value=make_response(200, spec_yaml)):
        strategy = RedocStrategy(detection)
        content = strategy.fetch_page("https://example.com/docs", {})
    assert content.openapi_spec["info"]["title"] == "Redoc API"
