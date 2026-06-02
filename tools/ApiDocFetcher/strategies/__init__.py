from __future__ import annotations

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
    request_body: bool = True   # matches Rust/TS field name
    responses: bool = True      # matches Rust/TS field name
    code_samples: bool = True

    @classmethod
    def from_dict(cls, d: dict) -> "KeepOptions":
        return cls(**{k: v for k, v in d.items() if k in cls.__dataclass_fields__})


class Strategy(Protocol):
    def fetch_tree(self, url: str, cookies: dict) -> list[DocNode]: ...
    def fetch_page(self, url: str, cookies: dict) -> PageContent: ...


def get_strategy(detection: Detection) -> Strategy:
    """Return the matching strategy instance for a Detection."""
    from .openapi_direct import OpenApiDirectStrategy
    from .mintlify_next import MintlifyNextStrategy
    from .swagger_ui import SwaggerUiStrategy
    from .redoc import RedocStrategy
    from .docusaurus import DocusaurusStrategy
    from .ai_generic import AiGenericStrategy

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
