"""workflow 結構檢查的測試。

這支檢查的存在理由是一次真實事故：拆分 release.yml 時刪除區塊的行號邊界抓錯，
把 finalize 最後一個步驟的 `run:` 一起砍掉，留下一個只有 name 與 env 的步驟。
那份 YAML 完全合法（`yaml.safe_load` 通過、job 圖印出來也正確），但 GitHub
Actions 視整個檔案為無效——每次 push 產生一個零 job 的失敗 run，而且錯誤訊息
只有「This run likely failed because of a workflow file issue」，不指出哪一行。
"""
import unittest

from check_workflows import find_problems


class FindProblems(unittest.TestCase):
    def test_a_step_with_neither_uses_nor_run_is_reported(self):
        # 這就是那次事故的形狀。
        wf = {
            "jobs": {
                "finalize": {
                    "steps": [
                        {"name": "Publish", "env": {"TAG": "x"}},
                    ]
                }
            }
        }
        problems = find_problems("release.yml", wf)
        self.assertEqual(len(problems), 1)
        self.assertIn("finalize", problems[0])
        self.assertIn("Publish", problems[0])

    def test_a_step_with_run_is_fine(self):
        wf = {"jobs": {"a": {"steps": [{"name": "x", "run": "echo hi"}]}}}
        self.assertEqual(find_problems("release.yml", wf), [])

    def test_a_step_with_uses_is_fine(self):
        wf = {"jobs": {"a": {"steps": [{"uses": "actions/checkout@v4"}]}}}
        self.assertEqual(find_problems("release.yml", wf), [])

    def test_a_job_referring_to_a_job_that_does_not_exist_is_reported(self):
        # 刪掉一個 job 卻忘了改別人的 needs——拆分 workflow 時的另一個形狀。
        # GitHub 對這個的回報一樣沒有行號。
        wf = {
            "jobs": {
                "build": {"steps": [{"run": "true"}]},
                "finalize": {"needs": ["build", "cli-build"], "steps": [{"run": "true"}]},
            }
        }
        problems = find_problems("release.yml", wf)
        self.assertEqual(len(problems), 1)
        self.assertIn("cli-build", problems[0])

    def test_needs_may_be_a_bare_string(self):
        wf = {
            "jobs": {
                "build": {"steps": [{"run": "true"}]},
                "finalize": {"needs": "build", "steps": [{"run": "true"}]},
            }
        }
        self.assertEqual(find_problems("release.yml", wf), [])

    def test_a_job_with_no_steps_and_no_reusable_workflow_is_reported(self):
        wf = {"jobs": {"a": {"runs-on": "ubuntu-latest"}}}
        problems = find_problems("release.yml", wf)
        self.assertEqual(len(problems), 1)
        self.assertIn("a", problems[0])

    def test_a_job_that_calls_a_reusable_workflow_needs_no_steps(self):
        wf = {"jobs": {"a": {"uses": "./.github/workflows/other.yml"}}}
        self.assertEqual(find_problems("release.yml", wf), [])


class TheRealWorkflows(unittest.TestCase):
    def test_every_workflow_in_this_repo_passes(self):
        from check_workflows import check_all

        self.assertEqual(check_all(), [])


if __name__ == "__main__":
    unittest.main()
