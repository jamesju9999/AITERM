"""Fail if ci.yml's warm-cache matrix drifts from release.yml's build matrix.

swatinem/rust-cache derives its key from the runner OS, the shared-key and the
Cargo.lock hash. If any of the six (os, target, shared-key) triples disagree,
that leg's cache is written under a key no release run will ever look up — and
the only symptom is that some legs stay slow, which reads as normal variance.
"""

import pathlib
import sys

import yaml

ROOT = pathlib.Path(__file__).resolve().parent.parent


def release_triples():
    wf = yaml.safe_load((ROOT / ".github/workflows/release.yml").read_text())
    return {
        (m["os"], m["rust_targets"], m["artifact_name"])
        for m in wf["jobs"]["build"]["strategy"]["matrix"]["include"]
    }


def ci_triples():
    wf = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())
    return {
        (m["os"], m["target"], m["shared_key"])
        for m in wf["jobs"]["warm-cache"]["strategy"]["matrix"]["include"]
    }


def main():
    release = release_triples()
    ci = ci_triples()
    if release == ci:
        print(f"matrices aligned ({len(ci)} legs)")
        return 0
    for triple in sorted(release - ci):
        print(f"in release.yml but not ci.yml: {triple}", file=sys.stderr)
    for triple in sorted(ci - release):
        print(f"in ci.yml but not release.yml: {triple}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
