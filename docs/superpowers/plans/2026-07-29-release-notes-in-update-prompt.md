# 更新提示中顯示更新項目 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 讓更新提示顯示人工確認過的變更清單，取代目前那串 release 網址。

**Architecture:** `create-release` 從 git log 產生草稿寫進 draft release body 的標記區塊；六條腿建置期間使用者在 GitHub 上改寫該區塊；`finalize` 停在 GitHub Environment 審核閘，核准後重讀 body、取出區塊、填進 `latest.json` 的 `notes` 再發布。所有解析邏輯放在 `scripts/release_notes.py` 的純函式裡，因為內嵌在 workflow 的邏輯無法測試，而本專案已因此吃過兩次虧。

**Tech Stack:** GitHub Actions、Python 3（標準庫 `unittest`，不新增相依）、React 19 + Vitest。

---

## 設計依據

規格：`docs/superpowers/specs/2026-07-29-release-notes-in-update-prompt-design.md`

實作者必須知道的既有事實（皆已查證，不要重新假設）：

- `UpdateModal.tsx:59` 已經在渲染 `state.notes`；`.update-modal-notes`（`UpdateModal.css:34-42`）已有 `white-space: pre-wrap`、`max-height: 120px`、`overflow-y: auto`。**多行內容不需要任何樣式改動。**
- `latest.json` 的 `notes` 目前寫死在 `.github/workflows/release.yml:535`。
- `create-release` 目前**沒有** checkout，只用 `actions/github-script@v7`。
- `finalize` 目前**沒有** checkout。
- `create-release` 在 release 已存在時走 reuse 路徑不覆寫 body（`release.yml:76-79`），因此人工編輯在重跑 workflow 時會存活。
- 這個 repo 只有 `release.yml` 一個 workflow，**沒有任何 PR 層級的測試閘**。所有測試都只在本機執行。
- repo 是 public，所以 environment 的 required reviewers 可免費使用。

## File Structure

| 檔案 | 責任 |
|---|---|
| `scripts/release_notes.py`（新增） | 兩件事的純函式與 CLI：從 commit 主旨產生草稿、從 release body 取出更新項目 |
| `scripts/test_release_notes.py`（新增） | 上者的 `unittest` 測試 |
| `.github/workflows/release.yml`（修改） | `create-release` 產生草稿並寫進 body；`finalize` 加審核閘並改用取出的內容 |
| `src/lib/repo.ts`（修改） | 新增 `releaseTagUrl` |
| `src/lib/repo.test.ts`（新增） | `releaseTagUrl` 的測試 |
| `src/lib/i18n.ts`（修改） | 新增 `update_view_full_notes` |
| `src/components/UpdateModal.tsx`（修改） | 新增【查看完整說明】按鈕 |
| `src/components/UpdateModal.css`（修改） | 該按鈕的樣式 |
| `src/components/UpdateModal.test.tsx`（修改） | 該按鈕的測試 |

---

## Task 1: 草稿產生的純函式

**Files:**
- Create: `scripts/release_notes.py`
- Test: `scripts/test_release_notes.py`

- [ ] **Step 1: 寫失敗的測試**

建立 `scripts/test_release_notes.py`：

```python
import unittest

from release_notes import filter_commits, render_draft

PLACEHOLDER = "- （本版無使用者可見的變更，請改寫此行）"


class FilterCommitsTest(unittest.TestCase):
    def test_keeps_feat_and_fix(self):
        self.assertEqual(
            filter_commits(["feat: add tabs", "fix: stop the crash"]),
            ["feat: add tabs", "fix: stop the crash"],
        )

    def test_keeps_scoped_forms(self):
        self.assertEqual(
            filter_commits(["feat(appimage): add a Settings section"]),
            ["feat(appimage): add a Settings section"],
        )

    def test_keeps_breaking_change_marker(self):
        self.assertEqual(filter_commits(["fix!: drop the old config"]), ["fix!: drop the old config"])
        self.assertEqual(
            filter_commits(["feat(ai)!: rename the provider field"]),
            ["feat(ai)!: rename the provider field"],
        )

    def test_drops_other_types(self):
        self.assertEqual(
            filter_commits(
                [
                    "chore: bump version to 1.2.4",
                    "docs: add the design spec",
                    "test(appimage): stub telegram_get_config",
                    "style: order the module declaration",
                    "refactor(ai): split the router",
                    "ci: pin appimagetool",
                ]
            ),
            [],
        )

    def test_drops_near_misses(self):
        # "feature" and "fixes" are not conventional-commit types. A prefix match
        # would wrongly keep both, and that mistake is invisible in a draft the
        # human is about to rewrite anyway — so it must be caught here.
        self.assertEqual(filter_commits(["feature: add tabs", "fixes: the crash"]), [])

    def test_drops_blank_lines(self):
        self.assertEqual(filter_commits(["", "   ", "feat: real"]), ["feat: real"])


class RenderDraftTest(unittest.TestCase):
    def test_prefixes_each_line_with_a_bullet(self):
        self.assertEqual(
            render_draft(["feat: add tabs", "fix: stop the crash"]),
            "- feat: add tabs\n- fix: stop the crash",
        )

    def test_empty_input_yields_the_placeholder(self):
        # Never an empty block: extract_changelog rejects one, which would block
        # the release with a confusing error instead of showing this line.
        self.assertEqual(render_draft([]), PLACEHOLDER)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `python3 -m unittest discover -s scripts -p 'test_*.py' -v`
Expected: FAIL — `ModuleNotFoundError: No module named 'release_notes'`

- [ ] **Step 3: 寫最小實作**

建立 `scripts/release_notes.py`：

```python
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
```

- [ ] **Step 4: 執行測試確認通過**

Run: `python3 -m unittest discover -s scripts -p 'test_*.py' -v`
Expected: PASS，8 個測試（`FilterCommitsTest` 6 個、`RenderDraftTest` 2 個）。數字若對不上，**以實際數量為準並回頭確認是否漏貼測試**，不要假設計畫寫的一定對。

- [ ] **Step 5: 驗證測試真的有效（mutation testing）**

逐一套用下表的改動，執行測試，確認**有測試失敗**，然後還原。

| # | 改動 | 必須失敗的測試 |
|---|---|---|
| M1 | `_KEEP` 改成 `r"^(feat\|fix)"` | `test_drops_near_misses` |
| M2 | `_KEEP` 拿掉 `!?` | `test_keeps_breaking_change_marker` |
| M3 | `_KEEP` 拿掉 `(\([^)]*\))?` 這一段 | `test_keeps_scoped_forms` |
| M4 | `render_draft` 的空清單改成 `return ""` | `test_empty_input_yields_the_placeholder` |
| M5 | `render_draft` 拿掉 `f"- {s}"` 的 `- ` | `test_prefixes_each_line_with_a_bullet` |

**不要**把「拿掉 `_KEEP` 開頭的 `^`」當成 mutation：`re.match` 本身就只從字串開頭比對，`^` 在此是冗餘的自我說明，拿掉不會改變任何行為，該 mutation 必然存活且不代表測試有缺口。

- [ ] **Step 6: Commit**

```bash
git add scripts/release_notes.py scripts/test_release_notes.py
git commit -m "feat(release): add pure helpers for changelog draft generation"
```

---

## Task 2: 從 release body 取出更新項目

**Files:**
- Modify: `scripts/release_notes.py`
- Modify: `scripts/test_release_notes.py`

- [ ] **Step 1: 寫失敗的測試**

在 `scripts/test_release_notes.py` 的 `import` 行改為：

```python
from release_notes import ChangelogError, extract_changelog, filter_commits, render_draft
```

並在 `if __name__ == "__main__":` 之前加入：

```python
BODY = """## AITerm v1.2.5

### 更新項目
<!-- changelog:start -->
- AppImage 可建立桌面選單項目
- 修正磁碟機切換的卡頓
<!-- changelog:end -->

### 下載
- **macOS**: 下載 `.dmg`
"""


class ExtractChangelogTest(unittest.TestCase):
    def test_returns_the_block_contents(self):
        self.assertEqual(
            extract_changelog(BODY),
            "- AppImage 可建立桌面選單項目\n- 修正磁碟機切換的卡頓",
        )

    def test_excludes_the_markers_themselves(self):
        self.assertNotIn("changelog:start", extract_changelog(BODY))
        self.assertNotIn("changelog:end", extract_changelog(BODY))

    def test_missing_start_marker_raises(self):
        body = BODY.replace("<!-- changelog:start -->", "")
        with self.assertRaises(ChangelogError):
            extract_changelog(body)

    def test_missing_end_marker_raises(self):
        body = BODY.replace("<!-- changelog:end -->", "")
        with self.assertRaises(ChangelogError):
            extract_changelog(body)

    def test_reversed_markers_raise(self):
        # Asserts the *reason*, not just the type. A start>stop slice returns ""
        # in Python rather than raising, so dropping the after_start argument to
        # find() still raises — via the empty-block check, for the wrong reason.
        # assertRaises(ChangelogError) alone passes either way and leaves the
        # search-start argument untested. (Found by mutation M2 surviving.)
        body = "<!-- changelog:end -->\n- x\n<!-- changelog:start -->"
        with self.assertRaisesRegex(ChangelogError, "missing <!-- changelog:end -->"):
            extract_changelog(body)

    def test_empty_block_raises(self):
        # An empty notes field ships a release whose update prompt says nothing,
        # with every job green. Failing loud is the whole point of this function.
        body = "<!-- changelog:start -->\n   \n<!-- changelog:end -->"
        with self.assertRaises(ChangelogError):
            extract_changelog(body)

    def test_uses_the_first_pair_when_the_body_has_several(self):
        body = (
            "<!-- changelog:start -->\nfirst\n<!-- changelog:end -->\n"
            "<!-- changelog:start -->\nsecond\n<!-- changelog:end -->"
        )
        self.assertEqual(extract_changelog(body), "first")
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `python3 -m unittest discover -s scripts -p 'test_*.py' -v`
Expected: FAIL — `ImportError: cannot import name 'ChangelogError'`

- [ ] **Step 3: 寫最小實作**

在 `scripts/release_notes.py` 的 `PLACEHOLDER` 之後加入：

```python
START = "<!-- changelog:start -->"
END = "<!-- changelog:end -->"


class ChangelogError(Exception):
    """The release body does not contain a usable changelog block."""
```

並在 `render_draft` 之後加入：

```python
def extract_changelog(body):
    """Return the text between the changelog markers.

    Raises rather than returning a fallback: a silent fallback would publish a
    release whose update prompt is blank or shows a bare URL, with every CI job
    green. Blocking the release is the recoverable failure; shipping is not.
    """
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
    return text
```

- [ ] **Step 4: 執行測試確認通過**

Run: `python3 -m unittest discover -s scripts -p 'test_*.py' -v`
Expected: PASS，15 個測試（前一個任務的 8 個加上 `ExtractChangelogTest` 的 7 個）。

- [ ] **Step 5: 驗證測試真的有效（mutation testing）**

| # | 改動 | 必須失敗的測試 |
|---|---|---|
| M1 | 三個 `raise ChangelogError(...)` 全改成 `return ""` | 四個 raise 測試 |
| M2 | `body.find(END, after_start)` 改成 `body.find(END)` | `test_reversed_markers_raise`（只有在該測試斷言錯誤訊息時才會失敗——用 `assertRaises(ChangelogError)` 會存活） |
| M3 | `body[after_start:end]` 改成 `body[start:end]` | `test_excludes_the_markers_themselves` |
| M4 | 拿掉 `if not text:` 那段 | `test_empty_block_raises` |
| M5 | `.strip()` 拿掉 | `test_returns_the_block_contents` |

**M1 是本任務的驗收關鍵。** 靜默回傳空字串會讓發版照常完成、`notes` 變空，而所有 job 都是綠的——與這個 repo 反覆吃虧的失敗模式完全相同。

- [ ] **Step 6: Commit**

```bash
git add scripts/release_notes.py scripts/test_release_notes.py
git commit -m "feat(release): extract the changelog block from a release body"
```

---

## Task 3: CLI 介面

**Files:**
- Modify: `scripts/release_notes.py`
- Modify: `scripts/test_release_notes.py`

workflow 只能呼叫指令，不能 import。這一層決定 exit code，而 exit code 正是「發版被擋下」與「發版帶著空 notes 出去」的分界。

- [ ] **Step 1: 寫失敗的測試**

在 `scripts/test_release_notes.py` 頂端的 `import unittest` 之後加入：

```python
import pathlib
import subprocess
import sys

SCRIPT = str(pathlib.Path(__file__).with_name("release_notes.py"))


def run_cli(args, stdin_text):
    return subprocess.run(
        [sys.executable, SCRIPT, *args],
        input=stdin_text,
        capture_output=True,
        text=True,
    )
```

並在 `if __name__ == "__main__":` 之前加入：

```python
class CliTest(unittest.TestCase):
    def test_draft_reads_stdin_and_writes_bullets(self):
        result = run_cli(["draft"], "feat: add tabs\nchore: bump version\n")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "- feat: add tabs")

    def test_draft_with_no_matching_commits_still_succeeds(self):
        result = run_cli(["draft"], "chore: bump version\n")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), PLACEHOLDER)

    def test_extract_writes_the_block(self):
        result = run_cli(["extract"], BODY)
        self.assertEqual(result.returncode, 0)
        self.assertIn("AppImage 可建立桌面選單項目", result.stdout)

    def test_extract_exits_1_when_markers_are_missing(self):
        # The workflow relies on this exit code to stop the release. If it ever
        # became 0, finalize would publish with whatever happened to be on stdout.
        result = run_cli(["extract"], "no markers here")
        self.assertEqual(result.returncode, 1)
        self.assertIn("changelog:start", result.stderr)

    def test_unknown_command_exits_2(self):
        result = run_cli(["bogus"], "")
        self.assertEqual(result.returncode, 2)
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `python3 -m unittest discover -s scripts -p 'test_*.py' -v`
Expected: FAIL — `test_draft_reads_stdin_and_writes_bullets` 得到 returncode 1（腳本沒有 `main`，執行後不產生輸出）

- [ ] **Step 3: 寫最小實作**

在 `scripts/release_notes.py` 檔尾加入：

```python
def main(argv):
    if len(argv) != 2:
        print("usage: release_notes.py {draft|extract}", file=sys.stderr)
        return 2
    command = argv[1]
    data = sys.stdin.read()
    if command == "draft":
        print(render_draft(filter_commits(data.splitlines())))
        return 0
    if command == "extract":
        try:
            print(extract_changelog(data))
        except ChangelogError as error:
            print(
                f"{error}\n"
                f"Restore both marker lines in the release body:\n"
                f"  {START}\n  ...\n  {END}\n"
                f"then re-run the finalize job.",
                file=sys.stderr,
            )
            return 1
        return 0
    print(f"unknown command: {command}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
```

- [ ] **Step 4: 執行測試確認通過**

Run: `python3 -m unittest discover -s scripts -p 'test_*.py' -v`
Expected: PASS，20 個測試（前兩個任務的 15 個加上 `CliTest` 的 5 個）。

- [ ] **Step 5: 驗證測試真的有效（mutation testing）**

| # | 改動 | 必須失敗的測試 |
|---|---|---|
| M1 | `extract` 的 `return 1` 改成 `return 0` | `test_extract_exits_1_when_markers_are_missing` |
| M2 | 未知指令的 `return 2` 改成 `return 0` | `test_unknown_command_exits_2` |
| M3 | `draft` 分支改成 `print(render_draft(data.splitlines()))`（不過濾） | `test_draft_reads_stdin_and_writes_bullets` |

- [ ] **Step 6: 手動確認一次真實資料**

```bash
git log --pretty=format:%s v1.2.3..v1.2.4 | python3 scripts/release_notes.py draft
```

Expected: 印出 8 行 `- feat(appimage): ...` / `- fix(...): ...`，不含任何 `chore:`、`docs:`、`test:`、`style:` 開頭的行。

- [ ] **Step 7: Commit**

```bash
git add scripts/release_notes.py scripts/test_release_notes.py
git commit -m "feat(release): add the release_notes CLI used by the workflow"
```

---

## Task 4: `create-release` 產生草稿

**Files:**
- Modify: `.github/workflows/release.yml`（`create-release` job，約 24-89 行）

- [ ] **Step 1: 新增 checkout 與草稿步驟**

在 `create-release` 的 `steps:` 底下、現有的 `- name: Create or reuse the draft release` **之前**插入：

```yaml
      # git log needs the full history and every tag to resolve the previous
      # release. The default shallow checkout has neither.
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Generate the changelog draft
        id: changelog
        env:
          TAG: ${{ github.ref_name }}
        run: |
          PREV=$(git describe --tags --abbrev=0 "$TAG^" 2>/dev/null || true)
          # No previous tag (first release): RANGE degrades to $TAG, covering
          # the whole history rather than failing.
          RANGE=${PREV:+$PREV..}$TAG
          echo "range: $RANGE"
          {
            echo "list<<CHANGELOG_EOF"
            git log --pretty=format:%s "$RANGE" | python3 scripts/release_notes.py draft
            echo "CHANGELOG_EOF"
          } >> "$GITHUB_OUTPUT"
```

- [ ] **Step 2: 把草稿插進 release body**

在 `RELEASE_BODY` 中，於 `## AITerm ${{ github.ref_name }}` 與 `### 下載` 之間插入：

```yaml
            ### 更新項目
            <!-- changelog:start -->
            ${{ steps.changelog.outputs.list }}
            <!-- changelog:end -->

```

改完後 `RELEASE_BODY` 的開頭應為：

```yaml
          RELEASE_BODY: |
            ## AITerm ${{ github.ref_name }}

            ### 更新項目
            <!-- changelog:start -->
            ${{ steps.changelog.outputs.list }}
            <!-- changelog:end -->

            ### 下載
            - **macOS** (Apple Silicon): 下載 `.dmg` 安裝檔
```

- [ ] **Step 3: 驗證 YAML 仍可解析**

Run:
```bash
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('yaml ok')"
```
Expected: `yaml ok`

若機器上沒有 PyYAML，改用 `npx --yes js-yaml .github/workflows/release.yml > /dev/null && echo 'yaml ok'`。

- [ ] **Step 4: 在本機模擬草稿步驟**

```bash
TAG=v1.2.4
PREV=$(git describe --tags --abbrev=0 "$TAG^" 2>/dev/null || true)
RANGE=${PREV:+$PREV..}$TAG
echo "range: $RANGE"
git log --pretty=format:%s "$RANGE" | python3 scripts/release_notes.py draft
```

Expected: `range: v1.2.3..v1.2.4`，接著 8 行 `- feat/fix ...`。

這一步只證明指令本身正確。**它無法證明 workflow 會通過**——`GITHUB_OUTPUT` 的多行語法、`${{ }}` 插值的縮排行為只有實跑才知道，見 Task 9。

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "feat(ci): write a changelog draft into the draft release body"
```

---

## Task 5: `finalize` 改用取出的內容並加上審核閘

**Files:**
- Modify: `.github/workflows/release.yml`（`finalize` job，約 473 行起）

- [ ] **Step 1: 加上 environment 與 checkout**

把 `finalize` 的開頭：

```yaml
  finalize:
    needs: build
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - name: Download updater signatures
```

改為：

```yaml
  finalize:
    needs: build
    runs-on: ubuntu-latest
    # Pauses here until a required reviewer approves, which is the window in
    # which the changelog block in the draft release body gets rewritten by hand.
    # WARNING: if this environment does not exist, or exists without required
    # reviewers, GitHub runs the job immediately and reports no error at all.
    # Verify with the gh api command in the plan, not by looking at the web UI.
    environment: release-approval
    # Raised defensively. Whether the approval wait counts toward timeout-minutes
    # is not something this plan verified, and a finalize cancelled mid-wait would
    # leave the release as a draft with no latest.json.
    timeout-minutes: 720
    steps:
      # finalize needs scripts/release_notes.py.
      - uses: actions/checkout@v4

      - name: Download updater signatures
```

- [ ] **Step 2: 在 compose 之前取出更新項目**

把 `- name: Compose latest.json` 的 `run:` 開頭：

```yaml
        run: |
          python3 - <<'PY'
```

改為：

```yaml
        run: |
          # Read the body now, not at create-release time: the whole point of the
          # approval gate above is that a human edited it in between.
          gh release view "$TAG" --json body -q .body > body.md
          # Exits 1 when the markers are missing or the block is empty, which
          # fails this job and leaves the release as a draft. The updater endpoint
          # resolves through releases/latest, so a draft reaches nobody.
          python3 scripts/release_notes.py extract < body.md > notes.txt
          python3 - <<'PY'
```

- [ ] **Step 3: 讓 manifest 使用它**

把：

```python
              "notes": f"https://github.com/{repo}/releases/tag/{tag}",
```

改為：

```python
              "notes": pathlib.Path("notes.txt").read_text().strip(),
```

`pathlib` 已在該區塊的 import 中，不需新增。

- [ ] **Step 4: 驗證 YAML 仍可解析**

Run:
```bash
python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml')); print('yaml ok')"
```
Expected: `yaml ok`

- [ ] **Step 5: 在本機模擬取出步驟**

用真實的 v1.2.4 body（它**沒有**標記）確認會失敗：

```bash
gh release view v1.2.4 --json body -q .body > /tmp/body.md
python3 scripts/release_notes.py extract < /tmp/body.md; echo "exit=$?"
```

Expected: exit=1，stderr 說明缺少 `<!-- changelog:start -->`。

再用一份有標記的確認會成功：

```bash
printf '## x\n<!-- changelog:start -->\n- 測試項目\n<!-- changelog:end -->\n' \
  | python3 scripts/release_notes.py extract; echo "exit=$?"
```

Expected: 印出 `- 測試項目`，exit=0。

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "feat(ci): gate finalize on approval and publish the edited changelog"
```

---

## Task 6: `releaseTagUrl`

**Files:**
- Modify: `src/lib/repo.ts`
- Create: `src/lib/repo.test.ts`

- [ ] **Step 1: 寫失敗的測試**

建立 `src/lib/repo.test.ts`：

```typescript
import { describe, expect, it } from "vitest";

import { GITHUB_RELEASES_URL, releaseTagUrl } from "./repo";

describe("releaseTagUrl", () => {
  it("points at the specific tag, not the latest release", () => {
    // The modal is telling the user about *this* version. Sending them to
    // /releases/latest would show a different release the moment a newer one
    // ships, which is exactly when this link matters most.
    expect(releaseTagUrl("1.2.5")).toBe(
      "https://github.com/jamesju9999/AITERM/releases/tag/v1.2.5",
    );
    expect(releaseTagUrl("1.2.5")).not.toBe(GITHUB_RELEASES_URL);
  });

  it("does not double the v prefix", () => {
    expect(releaseTagUrl("1.2.5")).toContain("/tag/v1.2.5");
    expect(releaseTagUrl("1.2.5")).not.toContain("/tag/vv");
  });
});
```

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/lib/repo.test.ts`
Expected: FAIL — `releaseTagUrl` 不存在

- [ ] **Step 3: 寫最小實作**

在 `src/lib/repo.ts` 檔尾加入：

```typescript
/** The release page for one specific version. `version` carries no "v" prefix. */
export function releaseTagUrl(version: string): string {
  return `${GITHUB_REPO_URL}/releases/tag/v${version}`;
}
```

- [ ] **Step 4: 執行測試確認通過**

Run: `npx vitest run src/lib/repo.test.ts`
Expected: PASS，2 個測試。

- [ ] **Step 5: Commit**

```bash
git add src/lib/repo.ts src/lib/repo.test.ts
git commit -m "feat(update): add a per-version release page URL"
```

---

## Task 7: 提示框的【查看完整說明】

**Files:**
- Modify: `src/lib/i18n.ts:356`（zh-TW）與 `src/lib/i18n.ts:1376`（en）
- Modify: `src/components/UpdateModal.tsx`
- Modify: `src/components/UpdateModal.css`
- Modify: `src/components/UpdateModal.test.tsx`

`en` 是 `{...zhTW, ...enRaw}`（`i18n.ts:1952-1958`），所以兩邊都要加，否則英文會落回中文。

- [ ] **Step 1: 寫失敗的測試**

在 `src/components/UpdateModal.test.tsx` 的 `describe("UpdateModalView", ...)` 內加入：

```typescript
  it("offers a link to the full release notes when available", async () => {
    const onOpenNotes = vi.fn();
    const props = renderView(
      { status: "available", version: "1.2.5", notes: "- 修正了一件事" },
      { onOpenNotes },
    );

    await userEvent.click(screen.getByRole("button", { name: "查看完整說明" }));
    // The view stays pure: it hands back the version and lets the container
    // build the URL, so the URL itself is covered by repo.test.ts.
    expect(props.onOpenNotes).toHaveBeenCalledWith("1.2.5");
  });

  it("still offers the link when the release carries no notes", () => {
    // An empty notes field is when the user most needs somewhere to look.
    renderView({ status: "available", version: "1.2.5", notes: "" }, { onOpenNotes: vi.fn() });

    expect(screen.getByRole("button", { name: "查看完整說明" })).toBeTruthy();
  });

  it("renders multi-line notes without collapsing them", () => {
    renderView(
      { status: "available", version: "1.2.5", notes: "- 第一項\n- 第二項" },
      { onOpenNotes: vi.fn() },
    );

    expect(screen.getByText(/第一項/)).toBeTruthy();
    expect(screen.getByText(/第二項/)).toBeTruthy();
  });

  it("does not offer the link while downloading", () => {
    renderView({ status: "downloading", downloaded: 1, total: 2 }, { onOpenNotes: vi.fn() });

    expect(screen.queryByRole("button", { name: "查看完整說明" })).toBeNull();
  });
```

並把檔案頂端的 `renderView` 輔助函式中的 props 物件加入 `onOpenNotes: vi.fn(),`（放在 `onOpenReleases: vi.fn(),` 之後），使其他既有測試不因缺少必要 prop 而報型別錯誤。

- [ ] **Step 2: 執行測試確認失敗**

Run: `npx vitest run src/components/UpdateModal.test.tsx`
Expected: FAIL — 找不到名為「查看完整說明」的按鈕

- [ ] **Step 3: 新增翻譯字串**

`src/lib/i18n.ts`，在 zh-TW 的 `about_update_link: "點此前往下載",` 之後加入：

```typescript
    update_view_full_notes: "查看完整說明",
```

在 en 的 `about_update_link: "Click to download",` 之後加入：

```typescript
    update_view_full_notes: "View full release notes",
```

- [ ] **Step 4: 修改元件**

`src/components/UpdateModal.tsx`：

在 `UpdateModalViewProps` 的 `onOpenReleases: () => void;` 之後加入：

```typescript
  onOpenNotes: (version: string) => void;
```

在函式解構參數 `onOpenReleases,` 之後加入：

```typescript
  onOpenNotes,
```

把現有的這一段：

```tsx
        {state.status === "available" && state.notes && (
          <p className="update-modal-notes">{state.notes}</p>
        )}
```

改為：

```tsx
        {state.status === "available" && state.notes && (
          <p className="update-modal-notes">{state.notes}</p>
        )}

        {state.status === "available" && (
          <button
            type="button"
            className="update-modal-notes-link"
            onClick={() => onOpenNotes(state.version)}
          >
            {t.update_view_full_notes}
          </button>
        )}
```

在檔案底部的 `UpdateModal` 容器中，`import` 區塊改為包含 `releaseTagUrl`：

```typescript
import { GITHUB_RELEASES_URL, releaseTagUrl } from "../lib/repo";
```

並在 `<UpdateModalView ... />` 的 props 中，於 `onOpenReleases={...}` 之後加入：

```tsx
      onOpenNotes={(version) => openUrl(releaseTagUrl(version)).catch(console.error)}
```

- [ ] **Step 5: 新增樣式**

`src/components/UpdateModal.css`，在 `.update-modal-notes { ... }` 規則之後加入：

```css
/* A text link rather than a button: it is a secondary escape hatch, not an
   action competing with 立即更新. `.update-modal` is a flex column, so
   align-self keeps it from stretching the full width. */
.update-modal-notes-link {
  align-self: flex-start;
  background: none;
  border: none;
  padding: 0;
  font: inherit;
  font-size: 12px;
  color: #7aadff;
  cursor: pointer;
  text-decoration: underline;
}
```

- [ ] **Step 6: 執行測試確認通過**

Run: `npx vitest run src/components/UpdateModal.test.tsx src/lib/repo.test.ts`
Expected: PASS，`UpdateModal.test.tsx` 既有 12 個加上新增 4 個共 16 個，另加 `repo.test.ts` 的 2 個。

- [ ] **Step 7: 驗證測試真的有效（mutation testing）**

| # | 改動 | 必須失敗的測試 |
|---|---|---|
| M1 | `onOpenNotes(state.version)` 改成 `onOpenNotes("")` | `offers a link to the full release notes when available` |
| M2 | 按鈕的顯示條件加上 `&& state.notes` | `still offers the link when the release carries no notes` |
| M3 | 按鈕的顯示條件改成 `state.status !== "idle"` | `does not offer the link while downloading` |
| M4 | `releaseTagUrl` 改成回傳 `GITHUB_RELEASES_URL` | `repo.test.ts` 的 `points at the specific tag` |

- [ ] **Step 8: 完整驗證**

```bash
npx tsc -b
npm run test
npx eslint $(git diff --name-only master...HEAD -- '*.ts' '*.tsx')
python3 -m unittest discover -s scripts -p 'test_*.py'
```

Expected: 全部乾淨。**不要用 `npx tsc --noEmit`**——根 `tsconfig.json` 是 solution 檔（`"files": []`），那條指令什麼都不檢查且永遠 exit 0，v1.2.4 六條腿全掛就是這樣漏過去的。

**不要跑 `npm run lint`** 作為關卡——既有約 181 個問題與本次無關。

- [ ] **Step 9: Commit**

```bash
git add src/lib/i18n.ts src/components/UpdateModal.tsx src/components/UpdateModal.css src/components/UpdateModal.test.tsx
git commit -m "feat(update): link to the full release notes from the update prompt"
```

---

## Task 8: 建立並驗證審核閘

**Files:** 無（GitHub 設定）

這一項無法由程式完成，且**它是整個設計最容易靜默失效的地方**。

- [ ] **Step 1: 請使用者建立 environment**

請使用者在 `https://github.com/jamesju9999/AITERM/settings/environments` 建立名為 `release-approval` 的 environment，勾選 **Required reviewers** 並把自己加進去。

**不要在這個 environment 放任何 secret。** 本專案先前有一個 `copilot` environment 存放更新私鑰副本，而 repo 層缺少同名 secret 時會展開成空字串，使錯誤訊息指向金鑰格式而非「沒設定」。簽章金鑰維持在 repo 層。

- [ ] **Step 2: 用 API 驗證，不要只看網頁**

```bash
gh api repos/jamesju9999/AITERM/environments/release-approval \
  -q '.protection_rules[] | select(.type=="required_reviewers") | .reviewers[].reviewer.login'
```

Expected: 印出 `jamesju9999`。

**沒有輸出即代表閘不存在或沒有審核者**，此時 GitHub 會讓 `finalize` 直接執行且不報任何錯，人工改寫的時間窗根本不存在。這與本專案先前「secret 設在 environment 層」的陷阱同類：設定沒生效，但外觀完全正常。

若指令回 404，代表 environment 尚未建立，回到 Step 1。

---

## Task 9: 下次發版的端到端驗證

**Files:** 無（實機驗證）

`GITHUB_OUTPUT` 的多行語法、`${{ }}` 插值進 YAML block scalar 的縮排行為、以及審核閘是否真的停下來，**都只有實際發版才能證明**。這個 repo 沒有 PR 層級的 CI，前面所有測試都只在本機跑。

- [ ] **Step 1: 發版前先問**

**未經使用者明確同意不得推 tag。** 推 `vX.Y.Z` 會觸發正式發版。

- [ ] **Step 2: 觀察 draft release**

`create-release` 完成後：

```bash
gh release view <TAG> --json body -q .body | head -20
```

Expected: 含「### 更新項目」、`<!-- changelog:start -->`、若干 `- feat/fix ...` 行、`<!-- changelog:end -->`，且區塊位於「### 下載」之前。

若草稿是空的或標記錯位，`${{ }}` 的插值行為與預期不符——記錄實際輸出再修正。

- [ ] **Step 3: 改寫並確認閘確實停住**

請使用者在 GitHub 上把該區塊改成繁中的使用者導向敘述。

同時確認 `finalize` **停在 Waiting 狀態**：

```bash
gh run view <RUN_ID> --json jobs -q '.jobs[] | select(.name=="finalize") | "\(.status) \(.conclusion)"'
```

Expected: `waiting ` 或 `queued `（尚未 approve）。

**若 `finalize` 直接跑完，代表閘沒生效**——回到 Task 8 Step 2 重新驗證，並把這次發版視為未驗證。

- [ ] **Step 4: 核准後比對內容**

使用者按 Approve，等 `finalize` 完成後：

```bash
curl -sL https://github.com/jamesju9999/AITERM/releases/latest/download/latest.json \
  | python3 -c 'import sys,json; print(json.load(sys.stdin)["notes"])'
```

Expected: 逐字等於使用者改寫後的內容。

- [ ] **Step 5: 在 App 內確認**

用舊版 App 觸發更新檢查，確認：

1. 提示框顯示多行更新項目，換行完整
2. 內容超過 120px 時可捲動
3.【查看完整說明】開啟的是該版本的 release 頁（`/releases/tag/v<版本>`），不是 `/releases/latest`

- [ ] **Step 6: 驗證失敗路徑**

在同一個 release 的 body 中刪掉 `<!-- changelog:end -->`，重跑 finalize：

```bash
gh run rerun --job <FINALIZE_JOB_ID>
```

Expected: job 失敗，log 中出現 `release body is missing <!-- changelog:end -->` 與修復指示。

**這一項不能跳過。** fail-loud 是這個設計的核心保障，而沒有實測過的錯誤處理等於沒有錯誤處理。驗證完把標記補回去。

這一步證明的是**錯誤路徑與訊息**。它**不能**證明「release 會留在 draft」——此時 release 早已由前一次成功的 finalize 發布出去，`latest.json` 也仍是好的。draft 保護只有在真正的首次發版失敗時才成立，本計畫無法安排該情境，必須在驗證結果中明確記為未驗證。

- [ ] **Step 7: 記錄結果**

在 `docs/superpowers/specs/2026-07-29-release-notes-in-update-prompt-design.md` 末尾附上「驗證結果」章節，寫明六項各自的結果、發版版本號，以及**明確列出未驗證的項目**。

```bash
git add -f docs/superpowers/specs/2026-07-29-release-notes-in-update-prompt-design.md
git commit -m "docs: record release-notes verification results"
```

---

## Notes for the implementer

- **綠燈不是證據。** Task 2 的 M1（`raise` 改成 `return ""`）是整份計畫最重要的一個 mutation：靜默回傳空字串會讓發版完成、`notes` 變空，而所有 job 都是綠的。
- **不要用 `npx tsc --noEmit`。** 根 `tsconfig.json` 是 solution 檔，那條指令永遠 exit 0。用 `npx tsc -b`。
- **不要跑 `npm run lint`** 作為關卡——既有約 181 個問題與本次無關。只 lint 本分支改動的檔案。
- **`docs/` 被 gitignore**（`.gitignore:47`），但 specs 與 plans 是追蹤的。用 `git add -f`。
- **絕不在未經同意下推 tag。**
- **不要改 `.update-modal-notes` 的樣式。** 它已有 `pre-wrap` / `max-height` / `overflow-y`，多行內容本來就能正確顯示；改動只會製造回歸。
