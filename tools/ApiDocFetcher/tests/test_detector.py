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
    # All calls (5 probes + 1 HTML fetch) must receive cookies
    assert mock_get.call_count >= 2, "Expected at least probe + HTML fetch calls"
    for c in mock_get.call_args_list:
        assert c.kwargs.get("cookies") == {"session": "abc"}, (
            f"Call missing cookies: {c}"
        )
