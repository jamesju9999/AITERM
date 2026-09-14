"""host-v* tag 的推導規則，給 .github/workflows/release-host.yml 用。

抽成一個模組而不是在 workflow 裡各寫一份 shell 判斷：npm、Homebrew、容器
三條對外管道都要知道「這一版是不是彩排版」，三份複製品一旦漂移，就會有一條
管道把彩排版發給所有使用者（或反過來，把正式版全部跳過）。而且寫在 YAML 裡
的判斷沒有任何辦法在發版之前測到。
"""

import sys

PREFIX = "host-v"


class InvalidTag(Exception):
    """The tag is not a host release tag."""


def version_for_tag(tag):
    """host-v0.2.0 -> 0.2.0。不是 host tag 就丟 InvalidTag。"""
    if not tag.startswith(PREFIX):
        raise InvalidTag(f"{tag!r} 不是 host 的 release tag（要以 {PREFIX!r} 開頭）")
    version = tag[len(PREFIX):]
    if not version:
        raise InvalidTag(f"{tag!r} 的前綴後面沒有版本號")
    return version


def is_prerelease(tag):
    """彩排／預發版本嗎？

    判斷的是**版本號裡**有沒有 "-"，不是整個 tag 裡有沒有 "-"。tag 的前綴
    host-v 自己就帶一個 "-"，用整個 tag 判斷會讓每一次正式發佈都被當成彩排。
    """
    return "-" in version_for_tag(tag)


def main(argv):
    if len(argv) != 3 or argv[1] not in ("version", "is-prerelease"):
        print(
            "usage: host_tag.py version <tag>\n"
            "       host_tag.py is-prerelease <tag>",
            file=sys.stderr,
        )
        return 2
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    command, tag = argv[1], argv[2]
    try:
        if command == "version":
            print(version_for_tag(tag))
        else:
            # shell 端直接拿去跟字串 'true' 比對，所以印小寫。
            print("true" if is_prerelease(tag) else "false")
    except InvalidTag as error:
        print(error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
