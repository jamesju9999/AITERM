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

START = "<!-- changelog:start -->"
END = "<!-- changelog:end -->"


class ChangelogError(Exception):
    """The release body does not contain a usable changelog block."""

# Conventional-commit types worth showing a user. Anchored and followed by an
# optional scope, an optional "!", then ":" — so "feature:" and "fixes:" do not
# match.
_KEEP = re.compile(r"^(feat|fix)(\([^)]*\))?!?:")


def filter_commits(subjects):
    """Keep only the commit subjects a user would care about."""
    stripped = (s.strip() for s in subjects)
    return [s for s in stripped if _KEEP.match(s)]


def _neutralise_markers(subject):
    """Stop a commit subject from closing the changelog block.

    "-->" terminates the HTML comment that ends the block, so a subject
    containing it would make extract_changelog stop early and publish a
    truncated changelog with exit 0 and every job green. Escaping the closer is
    enough: GitHub renders "--&gt;" as "-->", and a human rewrites the line anyway.
    """
    return subject.replace("-->", "--&gt;")


def section_for_tag(changelog, tag):
    """Return the CHANGELOG.md section for `tag`, or None if it has none.

    A section runs from its `## <tag>` heading to the next `## ` heading. The
    heading may carry a title after the version ("## v1.8.0 — 大改版").
    """
    if not changelog:
        return None
    changelog = changelog.replace("\r\n", "\n").replace("\r", "\n")
    # The version must be followed by a boundary, or a query for "v1.7.1"
    # would match the "## v1.7.12" heading (and ship the wrong section).
    heading = re.compile(
        r"^##\s+" + re.escape(tag) + r"(?:\s.*)?$",
        re.MULTILINE,
    )
    match = heading.search(changelog)
    if not match:
        return None
    rest = changelog[match.end():]
    next_heading = re.search(r"^##\s", rest, re.MULTILINE)
    section = rest[: next_heading.start()] if next_heading else rest
    section = section.strip()
    return section or None


def render_draft(kept, changelog_section=None):
    """Render the changelog block body. Never returns an empty string.

    A CHANGELOG.md section wins over commit subjects: the notes a user reads
    should be written and reviewed alongside the code, not retyped into the
    GitHub web editor at release time. Falls back to commit subjects so a
    release without a section still produces something to rewrite.
    """
    if changelog_section:
        return _neutralise_markers(changelog_section)
    if not kept:
        return PLACEHOLDER
    return "\n".join(f"- {_neutralise_markers(s)}" for s in kept)


def extract_changelog(body):
    """Return the text between the changelog markers.

    Raises rather than returning a fallback: a silent fallback would publish a
    release whose update prompt is blank or shows a bare URL, with every CI job
    green. Blocking the release is the recoverable failure; shipping is not.
    """
    # The block is rewritten in GitHub's web editor, so CRLF is the normal
    # input here. Left alone, every \r would travel into latest.json's notes
    # and then into a `white-space: pre-wrap` element in the update prompt.
    body = body.replace("\r\n", "\n").replace("\r", "\n")
    start = body.find(START)
    if start == -1:
        raise ChangelogError(f"release body is missing {START}")
    after_start = start + len(START)
    end = body.find(END, after_start)
    if end == -1:
        raise ChangelogError(f"release body is missing {END} after {START}")
    text = body[after_start:end].strip()
    if not text:
        raise ChangelogError("the changelog block is empty")
    if text == PLACEHOLDER:
        raise ChangelogError(
            "the changelog block still holds the generated placeholder — "
            "it was never rewritten"
        )
    return text


def main(argv):
    if len(argv) not in (2, 4) or (len(argv) == 4 and argv[1] != "draft"):
        print(
            "usage: release_notes.py extract\n"
            "       release_notes.py draft [<tag> <changelog-path>]",
            file=sys.stderr,
        )
        return 2
    # The placeholder and the extracted text are Traditional Chinese. Without
    # this, a non-UTF-8 locale turns either subcommand into a traceback with
    # exit 1 — the same exit code the workflow reads as "changelog is broken".
    sys.stdin.reconfigure(encoding="utf-8")
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    command = argv[1]
    if command == "draft":
        section = None
        if len(argv) == 4:
            tag, path = argv[2], argv[3]
            try:
                with open(path, encoding="utf-8") as handle:
                    section = section_for_tag(handle.read(), tag)
            except FileNotFoundError:
                # 沒有 CHANGELOG.md 就退回 commit 標題——不要因此擋住發版。
                section = None
            if section is None:
                print(
                    f"CHANGELOG.md 沒有 {tag} 的段落，改用 commit 標題產生草稿",
                    file=sys.stderr,
                )
        print(render_draft(filter_commits(sys.stdin.read().splitlines()), section))
        return 0
    if command == "extract":
        try:
            print(extract_changelog(sys.stdin.read()))
        except ChangelogError as error:
            print(
                f"{error}\n"
                f"Fix the changelog block in the release body, then re-run the\n"
                f"finalize job. It must sit between these two lines and describe\n"
                f"the release in the words a user would understand:\n"
                f"  {START}\n  ...\n  {END}",
                file=sys.stderr,
            )
            return 1
        return 0
    print(f"unknown command: {command}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
