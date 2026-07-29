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
