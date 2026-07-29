import unittest

from release_notes import ChangelogError, extract_changelog, filter_commits, render_draft

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
        # search-start argument untested.
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


if __name__ == "__main__":
    unittest.main()
