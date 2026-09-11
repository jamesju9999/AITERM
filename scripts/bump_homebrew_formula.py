#!/usr/bin/env python3
"""從一份 SHA256SUMS 產生 aiterm-host 的 Homebrew formula。

用法：
    python3 scripts/bump_homebrew_formula.py <version> <sums-file> <repo> > aiterm-host.rb
"""
import sys

# formula 需要的四個平台。Windows 的 zip 刻意不列——Homebrew 不裝那個。
PLATFORMS = {
    "arm_mac": "aarch64-apple-darwin",
    "intel_mac": "x86_64-apple-darwin",
    "arm_linux": "aarch64-unknown-linux-musl",
    "intel_linux": "x86_64-unknown-linux-musl",
}


def parse_sums(sums_text: str) -> dict:
    """把 `<sha>  <filename>` 每行解析成 {filename: sha}。"""
    out = {}
    for line in sums_text.splitlines():
        line = line.strip()
        if not line:
            continue
        sha, _, name = line.partition("  ")
        out[name.strip()] = sha.strip()
    return out


def render_formula(version: str, sums_text: str, repo: str) -> str:
    sums = parse_sums(sums_text)

    def sha_for(triple: str) -> str:
        key = f"aiterm-host-{version}-{triple}.tar.gz"
        # KeyError 而不是預設值：少一個平台就該當場失敗。悄悄漏掉的話，
        # 那個平台的使用者會拿到一個裝不起來的 formula。
        return sums[key]

    base = f"https://github.com/{repo}/releases/download/v{version}"

    def block(triple: str) -> str:
        return (
            f'      url "{base}/aiterm-host-{version}-{triple}.tar.gz"\n'
            f'      sha256 "{sha_for(triple)}"\n'
        )

    return f'''# 這個檔案由 scripts/bump_homebrew_formula.py 產生，不要手改。
class AitermHost < Formula
  desc "Headless host that shares a shell to AITerm for AI-driven remote control"
  homepage "https://github.com/{repo}"
  version "{version}"
  license "Apache-2.0"

  on_macos do
    on_arm do
{block(PLATFORMS["arm_mac"])}    end
    on_intel do
{block(PLATFORMS["intel_mac"])}    end
  end

  on_linux do
    on_arm do
{block(PLATFORMS["arm_linux"])}    end
    on_intel do
{block(PLATFORMS["intel_linux"])}    end
  end

  def install
    bin.install "aiterm-host"
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/aiterm-host --version")
  end
end
'''


def main() -> int:
    if len(sys.argv) != 4:
        print(__doc__, file=sys.stderr)
        return 2
    version, sums_path, repo = sys.argv[1], sys.argv[2], sys.argv[3]
    with open(sums_path, encoding="utf-8") as f:
        print(render_formula(version, f.read(), repo), end="")
    return 0


if __name__ == "__main__":
    sys.exit(main())
