# tools/ApiDocFetcher/converter.py
from __future__ import annotations
import json
import datetime
from strategies import KeepOptions


class _DateEncoder(json.JSONEncoder):
    """Serialize date/datetime objects that YAML parsers produce from ISO date strings."""
    def default(self, obj: object) -> object:
        if isinstance(obj, (datetime.date, datetime.datetime)):
            return obj.isoformat()
        return super().default(obj)


def _dumps(obj: object) -> str:
    return json.dumps(obj, indent=2, ensure_ascii=False, cls=_DateEncoder)


# ── $ref resolver ─────────────────────────────────────────────────────────────

def _resolve(obj: object, spec: dict, _depth: int = 0) -> object:
    """Recursively resolve local #/ $ref references. Depth-limited to avoid cycles."""
    if _depth > 8:
        return obj
    if isinstance(obj, dict):
        if "$ref" in obj:
            ref = obj["$ref"]
            if isinstance(ref, str) and ref.startswith("#/"):
                parts = ref[2:].split("/")
                resolved: object = spec
                for part in parts:
                    part = part.replace("~1", "/").replace("~0", "~")
                    if isinstance(resolved, dict):
                        resolved = resolved.get(part, {})
                    else:
                        return obj
                return _resolve(resolved, spec, _depth + 1)
            return obj  # external ref — leave as-is
        return {k: _resolve(v, spec, _depth + 1) for k, v in obj.items()}
    if isinstance(obj, list):
        return [_resolve(item, spec, _depth + 1) for item in obj]
    return obj


# ── Schema → Markdown table ───────────────────────────────────────────────────

def _schema_to_lines(schema: object, spec: dict, indent: int = 0) -> list[str]:
    """Render an OpenAPI schema dict as Markdown property rows (recursive for nested objects)."""
    schema = _resolve(schema, spec)
    if not isinstance(schema, dict):
        return []

    # allOf / oneOf / anyOf — merge first item
    for combiner in ("allOf", "oneOf", "anyOf"):
        if combiner in schema:
            items = schema[combiner]
            if items:
                return _schema_to_lines(items[0], spec, indent)

    props = schema.get("properties", {})
    if not props:
        # Array type — describe items
        if schema.get("type") == "array":
            items_schema = _resolve(schema.get("items", {}), spec)
            if isinstance(items_schema, dict) and "properties" in items_schema:
                return _schema_to_lines(items_schema, spec, indent)
        return []

    required_set = set(schema.get("required", []))
    prefix = "  " * indent

    if indent == 0:
        lines = [
            "| Field | Type | Required | Description |",
            "|-------|------|----------|-------------|",
        ]
    else:
        lines = []

    for name, prop in props.items():
        prop = _resolve(prop, spec)
        if not isinstance(prop, dict):
            continue
        ptype = prop.get("type", "")
        if not ptype:
            # Infer type from schema structure
            if "properties" in prop or "allOf" in prop:
                ptype = "object"
            elif "items" in prop:
                ptype = "array"
            else:
                ptype = "object"
        req_mark = "✓" if name in required_set else ""
        desc = str(prop.get("description", "")).replace("\r\n", " ").replace("\r", " ").replace("\n", " ").replace("|", "\\|")
        lines.append(f"| `{prefix}{name}` | {ptype} | {req_mark} | {desc} |")

        # Recurse one level for nested objects
        if indent < 1 and ptype in ("object", "array"):
            nested = _resolve(prop.get("items", prop), spec)
            nested_lines = _schema_to_lines(nested, spec, indent + 1)
            if indent == 0 and not lines[0].startswith("|"):
                pass
            lines.extend(nested_lines)

    return lines


def _render_schema(schema: object, spec: dict) -> list[str]:
    """Return Markdown lines for a schema. Falls back to JSON if no properties."""
    schema = _resolve(schema, spec)
    if not isinstance(schema, dict):
        return []
    rows = _schema_to_lines(schema, spec)
    if rows:
        return rows + [""]
    # Fallback: compact JSON (max 30 lines)
    raw = _dumps(schema)
    raw_lines = raw.split("\n")
    if len(raw_lines) > 30:
        raw_lines = raw_lines[:30] + ["  ..."]
    return ["```json"] + raw_lines + ["```", ""]


# ── Main converter ─────────────────────────────────────────────────────────────

def _render_security_schemes(spec: dict) -> list[str]:
    """Render securitySchemes from components into Markdown."""
    schemes = spec.get("components", {}).get("securitySchemes", {})
    if not schemes:
        return []

    lines = ["## Authentication", ""]
    for name, scheme in schemes.items():
        if not isinstance(scheme, dict):
            continue
        stype = scheme.get("type", "")
        lines += [f"### {name} (`{stype}`)", ""]

        if stype == "http":
            http_scheme = scheme.get("scheme", "")
            bearer_fmt = scheme.get("bearerFormat", "")
            lines.append(f"- **Scheme**: `{http_scheme}`")
            if bearer_fmt:
                lines.append(f"- **Format**: `{bearer_fmt}`")
            lines.append(f"- **Header**: `Authorization: {http_scheme.capitalize()} <token>`")
            lines.append("")

        elif stype == "oauth2":
            flows = scheme.get("flows", {})
            for flow_name, flow in flows.items():
                if not isinstance(flow, dict):
                    continue
                lines += [f"**Flow**: `{flow_name}`", ""]
                token_url = flow.get("tokenUrl", "")
                auth_url = flow.get("authorizationUrl", "")
                refresh_url = flow.get("refreshUrl", "")
                if token_url:
                    lines.append(f"- **Token URL**: `{token_url}`")
                if auth_url:
                    lines.append(f"- **Authorization URL**: `{auth_url}`")
                if refresh_url:
                    lines.append(f"- **Refresh URL**: `{refresh_url}`")
                scopes = flow.get("scopes", {})
                if scopes:
                    lines += ["", "**Scopes**:", ""]
                    lines += [
                        "| Scope | Description |",
                        "|-------|-------------|",
                    ]
                    for scope_name, scope_desc in scopes.items():
                        lines.append(f"| `{scope_name}` | {scope_desc} |")
                lines.append("")

        elif stype == "apiKey":
            lines.append(f"- **In**: `{scheme.get('in', '')}`")
            lines.append(f"- **Name**: `{scheme.get('name', '')}`")
            lines.append("")

        elif stype == "openIdConnect":
            lines.append(f"- **OpenID Connect URL**: `{scheme.get('openIdConnectUrl', '')}`")
            lines.append("")

    return lines


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

    # Security schemes (OAuth2 token URLs, API keys, etc.)
    security_lines = _render_security_schemes(spec)
    if security_lines:
        lines += security_lines

    lines += ["---", ""]

    paths = spec.get("paths", {})
    for path, path_item in paths.items():
        if not isinstance(path_item, dict):
            continue

        # Path-level parameters (shared across methods)
        path_params = [_resolve(p, spec) for p in path_item.get("parameters", [])]

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

            # Parameters — merge path-level + operation-level, resolve $ref
            if keep.parameters:
                op_params = [_resolve(p, spec) for p in op.get("parameters", [])]
                all_params: list[dict] = []
                seen_names: set[str] = set()
                for p in op_params + path_params:
                    if isinstance(p, dict):
                        n = p.get("name", "")
                        if n and n not in seen_names:
                            seen_names.add(n)
                            all_params.append(p)
                if all_params:
                    lines += [
                        "### Parameters",
                        "",
                        "| Name | In | Type | Required | Description |",
                        "|------|----|------|----------|-------------|",
                    ]
                    for p in all_params:
                        name = p.get("name", "")
                        loc = p.get("in", "")
                        required = "✓" if p.get("required") else ""
                        desc = str(p.get("description", "")).replace("\r\n", " ").replace("\r", " ").replace("\n", " ").replace("|", "\\|")
                        schema_obj = _resolve(p.get("schema", {}), spec)
                        ptype = ""
                        if isinstance(schema_obj, dict):
                            ptype = schema_obj.get("type", "")
                            if not ptype and "enum" in schema_obj:
                                ptype = "enum"
                        lines.append(f"| `{name}` | {loc} | {ptype} | {required} | {desc} |")
                    lines.append("")

            # Request body
            if keep.request_body:
                req_body = op.get("requestBody", {})
                if isinstance(req_body, dict):
                    content = req_body.get("content", {})
                    for media_type, media_obj in content.items():
                        if not isinstance(media_obj, dict):
                            continue
                        schema = media_obj.get("schema")
                        if schema:
                            lines += [
                                "### Request Body",
                                "",
                                f"Content-Type: `{media_type}`",
                                "",
                            ]
                            lines.extend(_render_schema(schema, spec))

                        # Render named examples (requestBody.content[].examples)
                        examples = media_obj.get("examples", {})
                        if isinstance(examples, dict) and examples:
                            lines += ["#### Examples", ""]
                            for ex_name, ex_obj in examples.items():
                                ex_obj = _resolve(ex_obj, spec) if isinstance(ex_obj, dict) else ex_obj
                                if not isinstance(ex_obj, dict):
                                    continue
                                summary = ex_obj.get("summary", ex_name)
                                lines += [f"**{summary}**", ""]
                                ex_desc = ex_obj.get("description", "")
                                if ex_desc:
                                    lines += [ex_desc.strip(), ""]
                                value = ex_obj.get("value")
                                if value is not None:
                                    raw = _dumps(value)
                                    lines += ["```json", raw, "```", ""]
                        elif "example" in media_obj:
                            # Single inline example
                            lines += ["#### Example", "", "```json",
                                      _dumps(media_obj["example"]),
                                      "```", ""]
                        break  # only first media type

            # Responses
            if keep.responses:
                responses = op.get("responses", {})
                if responses:
                    lines += [
                        "### Responses",
                        "",
                        "| Code | Description |",
                        "|------|-------------|",
                    ]
                    for code, resp in responses.items():
                        resp = _resolve(resp, spec) if isinstance(resp, dict) else {}
                        desc = (resp.get("description", "") if isinstance(resp, dict) else "").replace("|", "\\|")
                        lines.append(f"| {code} | {desc} |")
                    lines.append("")

                    # Show response schema + examples for 200
                    resp_200 = _resolve(responses.get("200", {}), spec)
                    if isinstance(resp_200, dict):
                        for media_type, media_obj in resp_200.get("content", {}).items():
                            if not isinstance(media_obj, dict):
                                continue
                            if media_obj.get("schema"):
                                lines += ["**200 Response Schema**", ""]
                                lines.extend(_render_schema(media_obj["schema"], spec))
                            # Response examples
                            ex_map = media_obj.get("examples", {})
                            if isinstance(ex_map, dict) and ex_map:
                                lines += ["**200 Response Examples**", ""]
                                for ex_name, ex_obj in ex_map.items():
                                    ex_obj = _resolve(ex_obj, spec) if isinstance(ex_obj, dict) else ex_obj
                                    if not isinstance(ex_obj, dict):
                                        continue
                                    summary = ex_obj.get("summary", ex_name)
                                    lines += [f"*{summary}*", ""]
                                    value = ex_obj.get("value")
                                    if value is not None:
                                        lines += ["```json",
                                                  _dumps(value),
                                                  "```", ""]
                            elif "example" in media_obj:
                                lines += ["**200 Response Example**", "", "```json",
                                          _dumps(media_obj["example"]),
                                          "```", ""]
                            break

            # Code samples (x-codeSamples extension)
            if keep.code_samples:
                for sample in (op.get("x-codeSamples") or []):
                    lang = sample.get("lang", "bash")
                    src = sample.get("source", "")
                    lines += [f"### Example ({lang})", "", f"```{lang.lower()}", src, "```", ""]

            lines += ["---", ""]

    return "\n".join(lines)
