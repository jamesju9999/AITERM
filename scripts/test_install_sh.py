"""install.sh 的目標三元組推導測試。

平台偵測寫錯的後果特別難察覺：使用者會下載到一個給別的架構用的執行檔，錯誤訊息
是 "cannot execute binary file" 這種跟根因無關的東西。所以這一段值得單獨釘住。
"""
import subprocess
import unittest
from pathlib import Path

SCRIPT = Path(__file__).parent / "install.sh"


def detect(os_name: str, arch: str) -> str:
    """用假的 uname 值呼叫 install.sh 的 detect_target。"""
    result = subprocess.run(
        ["bash", "-c", f'source "{SCRIPT}" --source-only; detect_target "{os_name}" "{arch}"'],
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip())
    return result.stdout.strip()


def pick_version(releases_json: str) -> str:
    """把 releases API 的 JSON 餵給 install.sh 的 pick_host_version。"""
    result = subprocess.run(
        ["bash", "-c", f'source "{SCRIPT}" --source-only; pick_host_version'],
        input=releases_json,
        capture_output=True,
        text=True,
    )
    return result.stdout.strip()


class DetectTarget(unittest.TestCase):
    def test_apple_silicon(self):
        self.assertEqual(detect("Darwin", "arm64"), "aarch64-apple-darwin")

    def test_intel_mac(self):
        self.assertEqual(detect("Darwin", "x86_64"), "x86_64-apple-darwin")

    def test_linux_x86_64_uses_musl(self):
        # 一律給 musl 靜態版：它在 glibc 系統上也跑得動，反過來不成立。
        self.assertEqual(detect("Linux", "x86_64"), "x86_64-unknown-linux-musl")

    def test_linux_arm64(self):
        self.assertEqual(detect("Linux", "aarch64"), "aarch64-unknown-linux-musl")

    def test_linux_arm64_reported_as_arm64(self):
        # 有些系統的 uname -m 回 arm64 而不是 aarch64。兩種都要接。
        self.assertEqual(detect("Linux", "arm64"), "aarch64-unknown-linux-musl")

    def test_unsupported_arch_is_an_error_not_a_guess(self):
        # 猜錯架構會讓使用者下載到跑不起來的執行檔，錯誤訊息還跟根因無關。
        # 寧可當場說「不支援」。
        with self.assertRaises(RuntimeError):
            detect("Linux", "riscv64")

    def test_unsupported_os_is_an_error(self):
        with self.assertRaises(RuntimeError):
            detect("FreeBSD", "x86_64")


# 依 GitHub API 的順序：最新的在最前面。刻意把桌面版的 v1.26.0 放在第一個、
# 把彩排版 host-v0.2.1-dist1 放在正式版前面——只取「第一個 tag_name」或
# 只剝前綴不看後綴的實作都會在這份資料上拿到錯的答案。
RELEASES_JSON = """[
  {"tag_name": "v1.26.0", "draft": false, "prerelease": false},
  {"tag_name": "host-v0.2.1-dist1", "draft": false, "prerelease": true},
  {"tag_name": "host-v0.2.0", "draft": false, "prerelease": false},
  {"tag_name": "v1.25.1", "draft": false, "prerelease": false},
  {"tag_name": "host-v0.1.9", "draft": false, "prerelease": false}
]"""


class PickHostVersion(unittest.TestCase):
    # 每個測試都用正向斷言（assertEqual）。用 assertNotIn("dist1", ...) 這種
    # 寫法的話，一個什麼都不做、永遠回空字串的實作也會通過——那等於沒測。

    def test_skips_desktop_releases(self):
        # 桌面版的 release 沒有 aiterm-host 的資產，抓到它的話下載會 404。
        # v1.26.0 排在最前面，只取「第一個 tag_name」的實作會在這裡出局。
        self.assertEqual(pick_version(RELEASES_JSON), "0.2.0")

    def test_skips_rehearsal_tags(self):
        # host-v0.2.1-dist1 比 host-v0.2.0 新，但它是彩排版，不該裝給使用者。
        # 只剝前綴、不看後綴的實作會在這裡回 "0.2.1-dist1"。
        only_rehearsal_is_newer = """[
          {"tag_name": "host-v0.2.1-dist1", "draft": false, "prerelease": true},
          {"tag_name": "host-v0.2.0", "draft": false, "prerelease": false}
        ]"""
        self.assertEqual(pick_version(only_rehearsal_is_newer), "0.2.0")

    def test_takes_the_newest_not_just_any(self):
        # 兩個都是正式版，要拿排在前面（比較新）的那一個。
        two_real_versions = """[
          {"tag_name": "host-v0.3.0", "draft": false, "prerelease": false},
          {"tag_name": "host-v0.1.9", "draft": false, "prerelease": false}
        ]"""
        self.assertEqual(pick_version(two_real_versions), "0.3.0")

    def test_no_host_release_yields_nothing(self):
        only_desktop = '[{"tag_name": "v1.26.0", "draft": false, "prerelease": false}]'
        self.assertEqual(pick_version(only_desktop), "")


if __name__ == "__main__":
    unittest.main()
