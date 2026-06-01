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
