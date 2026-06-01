# tools/ApiDocFetcher/tests/conftest.py
from unittest.mock import MagicMock


def make_response(status: int = 200, text: str = "") -> MagicMock:
    """Factory for mocked curl_cffi responses."""
    r = MagicMock()
    r.status_code = status
    r.text = text
    return r
