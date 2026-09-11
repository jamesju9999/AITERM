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


if __name__ == "__main__":
    unittest.main()
