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


def test_keep_request_schema_false_omits_request_body():
    keep = KeepOptions(request_schema=False)
    md = openapi_to_markdown(SIMPLE_SPEC, keep)
    assert "### Request Body" not in md


def test_keep_description_false_omits_description():
    keep = KeepOptions(description=False)
    md = openapi_to_markdown(SIMPLE_SPEC, keep)
    assert "> A test API." not in md


def test_keep_code_samples_false_omits_samples():
    spec_with_samples = {
        "openapi": "3.0.0",
        "info": {"title": "Sample API", "version": "1.0"},
        "paths": {
            "/test": {
                "get": {
                    "summary": "Test",
                    "responses": {"200": {"description": "OK"}},
                    "x-codeSamples": [{"lang": "Python", "source": "import requests"}],
                }
            }
        },
    }
    keep_with = KeepOptions(code_samples=True)
    keep_without = KeepOptions(code_samples=False)
    md_with = openapi_to_markdown(spec_with_samples, keep_with)
    md_without = openapi_to_markdown(spec_with_samples, keep_without)
    assert "### Example (Python)" in md_with
    assert "### Example (Python)" not in md_without
