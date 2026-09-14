#!/usr/bin/env sh
# AITerm CLI Host 安裝腳本。
#
#   curl -fsSL https://raw.githubusercontent.com/jamesju9999/AITERM/master/scripts/install.sh | sh
#
# 做四件事：偵測平台 → 抓對應的 asset → 用 SHA256SUMS 驗 checksum → 裝進
# ~/.local/bin。**checksum 不通過就中止**，不給「要不要照樣裝」的選項——這是一個
# 會拿到 shell 的執行檔。
set -eu

REPO="jamesju9999/AITERM"
INSTALL_DIR="${AITERM_HOST_INSTALL_DIR:-$HOME/.local/bin}"

detect_target() {
  os="$1"
  arch="$2"
  case "$os" in
    Darwin)
      case "$arch" in
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        x86_64) echo "x86_64-apple-darwin" ;;
        *) echo "不支援的 macOS 架構：$arch" >&2; return 1 ;;
      esac
      ;;
    Linux)
      # 一律給 musl 靜態版。它在 glibc 系統上也跑得動，反過來不成立——同一個
      # asset 就能涵蓋 alpine、distroless 與 glibc 太舊的老伺服器。
      case "$arch" in
        x86_64|amd64) echo "x86_64-unknown-linux-musl" ;;
        aarch64|arm64) echo "aarch64-unknown-linux-musl" ;;
        *) echo "不支援的 Linux 架構：$arch" >&2; return 1 ;;
      esac
      ;;
    *)
      echo "不支援的作業系統：${os}（Windows 請用 install.ps1）" >&2
      return 1
      ;;
  esac
}

# 從 /repos/{repo}/releases 的 JSON 裡挑出最新的正式 aiterm-host 版本號。
#
# 不能用 releases/latest：一個 repo 只有一個 latest，而它屬於桌面版 AITerm
# （桌面版的自動更新端點就指向 releases/latest/download/latest.json）。桌面版
# 發一次版，latest 就是一則沒有任何 aiterm-host 資產的 release，照著抓只會 404。
#
# 不解析 draft／prerelease 欄位，改用 tag 的形狀判斷：未登入的 API 本來就
# 看不到 draft，而這個 repo 的彩排 tag 一律是 host-v<版本>-<主題><n>，
# 所以「版本號只有數字和點」就等於「正式版」。grep 的樣式把結尾的引號一起
# 吃進去，host-v0.2.0-dist1 因此不會 match——在 sh 裡手刻 JSON 解析比這脆弱得多。
#
# API 回傳的順序是新到舊，所以第一個符合的就是最新的正式版。
pick_host_version() {
  grep -o '"tag_name"[[:space:]]*:[[:space:]]*"host-v[0-9][0-9.]*"' \
    | head -1 \
    | sed 's/.*"host-v\([0-9][0-9.]*\)"$/\1/'
}

main() {
  target="$(detect_target "$(uname -s)" "$(uname -m)")"

  echo "正在查最新版本…"
  version="$(curl -fsSL "https://api.github.com/repos/$REPO/releases?per_page=30" \
    | pick_host_version)"
  if [ -z "$version" ]; then
    echo "查不到 aiterm-host 的版本。GitHub API 可能限流了，或還沒有任何" >&2
    echo "host-v* 的正式 release。" >&2
    exit 1
  fi
  echo "最新版本：$version"

  name="aiterm-host-${version}-${target}"
  base="https://github.com/$REPO/releases/download/host-v${version}"

  tmp="$(mktemp -d)"
  # 中途失敗也要清乾淨，不要在 /tmp 留一堆半殘的下載。
  trap 'rm -rf "$tmp"' EXIT

  echo "下載 ${name}.tar.gz…"
  curl -fsSL -o "$tmp/$name.tar.gz" "$base/$name.tar.gz"
  curl -fsSL -o "$tmp/SHA256SUMS" "$base/aiterm-host-${version}-SHA256SUMS"

  # macOS 沒有 sha256sum，那邊叫 shasum -a 256。兩個都找不到就中止——
  # 「驗不了就照樣裝」對一個會拿到 shell 的執行檔是不能接受的。
  if command -v sha256sum >/dev/null 2>&1; then
    sumcmd="sha256sum"
  elif command -v shasum >/dev/null 2>&1; then
    sumcmd="shasum -a 256"
  else
    echo "找不到 sha256sum 或 shasum，無法驗證下載內容——中止。" >&2
    exit 1
  fi

  echo "驗證 checksum…"
  ( cd "$tmp" && grep " $name.tar.gz\$" SHA256SUMS | $sumcmd -c - ) || {
    echo "checksum 不符——中止安裝。下載的檔案可能被竄改或損毀。" >&2
    exit 1
  }

  tar xzf "$tmp/$name.tar.gz" -C "$tmp"
  mkdir -p "$INSTALL_DIR"
  install -m 755 "$tmp/$name/aiterm-host" "$INSTALL_DIR/aiterm-host"

  echo "已安裝：$INSTALL_DIR/aiterm-host"
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "提醒：$INSTALL_DIR 不在 PATH 裡。請加上：export PATH=\"\$PATH:$INSTALL_DIR\"" ;;
  esac
  echo "下一步：aiterm-host --print-connection"
}

# 測試要能只載入函式而不執行安裝。
case "${1:-}" in
  --source-only) return 0 2>/dev/null || exit 0 ;;
esac

main "$@"
