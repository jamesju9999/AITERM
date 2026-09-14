"""host-v* tag 的推導規則。

這一段值得單獨釘住，因為它的失敗是靜默的：npm／Homebrew／容器三條對外管道
都靠 is_prerelease 決定要不要發佈，判錯的結果是三條管道全部安靜跳過而 job
全綠——沒有任何訊號告訴你這一版根本沒發出去。
"""
import unittest

from host_tag import InvalidTag, is_prerelease, version_for_tag


class VersionForTag(unittest.TestCase):
    def test_strips_the_host_prefix(self):
        self.assertEqual(version_for_tag("host-v0.2.0"), "0.2.0")

    def test_keeps_the_rehearsal_suffix(self):
        # 彩排版的 asset 檔名要跟著帶後綴，否則兩次彩排會互相覆蓋。
        self.assertEqual(version_for_tag("host-v0.2.0-dist1"), "0.2.0-dist1")

    def test_a_desktop_tag_is_rejected(self):
        # 桌面版的 tag 誤觸發 host workflow 時要當場炸掉，不要安靜地發出一版
        # 版本號是 "1.26.0" 的 host。
        with self.assertRaises(InvalidTag):
            version_for_tag("v1.26.0")

    def test_a_bare_version_is_rejected(self):
        with self.assertRaises(InvalidTag):
            version_for_tag("0.2.0")


class IsPrerelease(unittest.TestCase):
    def test_a_release_tag_is_not_a_prerelease(self):
        # **這就是那個 bug。** 舊的判斷是 `[[ "$TAG" == *-* ]]`，而
        # "host-v0.2.0" 本身就含有 "-"，會讓每一次正式發佈都被跳過。
        self.assertFalse(is_prerelease("host-v0.2.0"))

    def test_a_rehearsal_tag_is_a_prerelease(self):
        self.assertTrue(is_prerelease("host-v0.2.0-dist1"))

    def test_a_semver_prerelease_is_a_prerelease(self):
        self.assertTrue(is_prerelease("host-v1.0.0-rc1"))

    def test_a_four_part_version_is_not_a_prerelease(self):
        # 版本號裡的點不是分隔符，不該被誤判。
        self.assertFalse(is_prerelease("host-v0.2.0.1"))


if __name__ == "__main__":
    unittest.main()
