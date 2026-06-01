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
