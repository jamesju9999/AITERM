"""Homebrew formula 產生器的測試。

formula 寫錯的失效模式很安靜：`brew install` 會下載到錯的 URL 或裝到舊版，
而使用者只會覺得「怎麼裝起來不是新的」。
"""
import unittest

from bump_homebrew_formula import render_formula


SUMS = """\
aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111  aiterm-host-1.25.0-aarch64-apple-darwin.tar.gz
bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222  aiterm-host-1.25.0-x86_64-apple-darwin.tar.gz
cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333cccc3333  aiterm-host-1.25.0-x86_64-unknown-linux-musl.tar.gz
dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444dddd4444  aiterm-host-1.25.0-aarch64-unknown-linux-musl.tar.gz
eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555eeee5555  aiterm-host-1.25.0-x86_64-pc-windows-msvc.zip
"""


class RenderFormula(unittest.TestCase):
    def setUp(self):
        self.out = render_formula("1.25.0", SUMS, "jamesju9999/AITERM")

    def test_version_appears(self):
        self.assertIn('version "1.25.0"', self.out)

    def test_each_platform_gets_its_own_sha(self):
        # 四個 sha 必須各就各位。貼錯位置的話 brew 會在下載後才驗出不符，
        # 錯誤訊息完全不會指向 formula。
        self.assertIn("aaaa1111", self.out)
        self.assertIn("bbbb2222", self.out)
        self.assertIn("cccc3333", self.out)
        self.assertIn("dddd4444", self.out)

    def test_the_windows_zip_is_not_included(self):
        # Homebrew 不裝 Windows 的東西。把它寫進去只會讓 formula 多一段死程式碼。
        self.assertNotIn("eeee5555", self.out)
        self.assertNotIn("windows", self.out)

    def test_arm_mac_sha_is_paired_with_the_arm_mac_url(self):
        # 這是真正會出錯的地方，而且有兩種不同形狀：
        #   (a) url 和 sha 本身配對錯位（url 是 arm 的，sha 卻是 intel 的）
        #   (b) 整個 platform 標籤點錯區塊（`on_arm do` 裡面裝的是 intel 的
        #       url+sha，只是這一對本身彼此仍是「自洽」的一致組合）
        #
        # 只比「這個 url 附近有沒有出現 aaaa1111」抓不到 (b)：因為 render_formula
        # 內部 sha 是照 url 自己的 triple 算出來的，url/sha 永遠自洽，就算
        # PLATFORMS 表裡 arm_mac / intel_mac 的值被整組對調，每個 url 底下
        # 貼著的 sha 依然「看起來正確」——只是被搬到了錯的 on_arm/on_intel 區塊。
        # 所以要先定位到結構上的 `on_arm do ... end` 區塊本身（macOS 的
        # on_arm 一定是檔案裡第一個出現的 "on_arm do"，因為 on_macos 區塊
        # 排在 on_linux 之前），再檢查區塊「裡面」裝的是不是 arm mac 那一組。
        start = self.out.index("on_arm do")
        end = self.out.index("end", start)
        block = self.out[start:end]

        self.assertIn(
            "aarch64-apple-darwin.tar.gz", block,
            "macOS 的 on_arm 區塊裡沒有 arm mac 的 url——platform 標籤點錯區塊了",
        )
        self.assertIn(
            "aaaa1111", block,
            "macOS 的 on_arm 區塊裡沒有 arm mac 的 sha",
        )
        self.assertNotIn(
            "bbbb2222", block,
            "macOS 的 on_arm 區塊裡混進了 intel mac 的 sha——配對錯位了",
        )
        self.assertNotIn(
            "x86_64-apple-darwin.tar.gz", block,
            "macOS 的 on_arm 區塊裡裝的是 intel mac 的 url",
        )

    def test_a_missing_platform_is_an_error_not_a_silent_omission(self):
        partial = "\n".join(SUMS.splitlines()[:2]) + "\n"
        with self.assertRaises(KeyError):
            render_formula("1.25.0", partial, "jamesju9999/AITERM")


if __name__ == "__main__":
    unittest.main()
