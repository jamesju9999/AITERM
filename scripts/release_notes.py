"""Release-notes helpers for .github/workflows/release.yml.

Kept out of the workflow as importable functions because logic embedded in a
workflow cannot be tested before a release. This repo has shipped a CI change
that was a silent no-op (`uploadUpdaterJson`, which is not a real tauri-action
input) and one that would have failed every release (a relative path used
inside a `cd` subshell) — neither was detectable by `bash -n` or YAML parsing.
"""

import re
import sys

PLACEHOLDER = "- （本版無使用者可見的變更，請改寫此行）"

# Conventional-commit types worth showing a user. Anchored and followed by an
# optional scope, an optional "!", then ":" — so "feature:" and "fixes:" do not
# match.
_KEEP = re.compile(r"^(feat|fix)(\([^)]*\))?!?:")


def filter_commits(subjects):
    """Keep only the commit subjects a user would care about."""
    return [s for s in subjects if _KEEP.match(s.strip())]


def render_draft(kept):
    """Render the changelog block body. Never returns an empty string."""
    if not kept:
        return PLACEHOLDER
    return "\n".join(f"- {s}" for s in kept)
