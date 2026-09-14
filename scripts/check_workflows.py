"""Fail if a workflow file is structurally invalid in ways YAML parsing misses.

`yaml.safe_load` succeeding proves nothing about whether GitHub Actions will
accept the file. A step with a `name` and an `env` but no `run` is perfectly
valid YAML and even lets you print the job graph — but Actions rejects the
whole file, and the only feedback is a zero-job run whose entire error message
is "This run likely failed because of a workflow file issue". No file, no line,
no key. That happened here while splitting release.yml: a delete-by-line-number
took the last step's `run:` block with it.

These checks are deliberately narrow. They cover the mistakes that editing a
1000-line workflow actually produces — a truncated step, a `needs` pointing at
a job that was moved to another file — not a general schema validation.
"""

import pathlib
import sys

import yaml

ROOT = pathlib.Path(__file__).resolve().parent.parent
WORKFLOW_DIR = ROOT / ".github/workflows"


def find_problems(path, workflow):
    """Return a list of human-readable problems in one parsed workflow."""
    problems = []
    jobs = workflow.get("jobs") or {}
    for job_name, job in jobs.items():
        job = job or {}

        # A job either has steps or calls a reusable workflow.
        steps = job.get("steps")
        if steps is None:
            if "uses" not in job:
                problems.append(
                    f"{path} :: job '{job_name}' 既沒有 steps 也沒有 uses"
                )
            continue

        for index, step in enumerate(steps):
            step = step or {}
            if "uses" not in step and "run" not in step:
                label = step.get("name", f"<第 {index + 1} 個步驟>")
                problems.append(
                    f"{path} :: job '{job_name}' 的步驟 '{label}' "
                    f"既沒有 uses 也沒有 run"
                )

        # `needs` must name jobs that exist in this same file.
        needs = job.get("needs") or []
        if isinstance(needs, str):
            needs = [needs]
        for required in needs:
            if required not in jobs:
                problems.append(
                    f"{path} :: job '{job_name}' 的 needs 指向不存在的 job "
                    f"'{required}'"
                )
    return problems


def check_all():
    """Check every workflow in .github/workflows. Returns a list of problems."""
    problems = []
    for path in sorted(WORKFLOW_DIR.glob("*.yml")) + sorted(
        WORKFLOW_DIR.glob("*.yaml")
    ):
        workflow = yaml.safe_load(path.read_text(encoding="utf-8"))
        problems.extend(find_problems(path.name, workflow))
    return problems


def main():
    problems = check_all()
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        print(
            f"\n{len(problems)} 個問題。這些形狀的錯誤 GitHub 只會回報成 "
            f'"This run likely failed because of a workflow file issue"，'
            f"不指出檔名或行號。",
            file=sys.stderr,
        )
        return 1
    print("workflow 結構檢查通過")
    return 0


if __name__ == "__main__":
    sys.exit(main())
