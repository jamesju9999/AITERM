#!/bin/sh
# 把 AITerm 登記成 x-terminal-emulator 的候選。priority 10 低於所有常見終端機
# （xterm/kitty/foot 是 20，DE 的終端機是 30–50），所以絕不會被自動選為預設，
# 也不會讓 sensible-terminal 之類的工具悄悄改用 AITerm；要用的人自己執行
# `update-alternatives --config x-terminal-emulator` 切換。
# 若 AITerm 是唯一的候選，仍會被自動選上（這是 alternatives 的正常行為）。
set -e

[ "$1" = "configure" ] || exit 0

# 執行檔名稱以套件實際安裝的 .desktop 的 Exec= 為準，不寫死
# （/usr/bin 下還有 uv 等 sidecar，不能隨便取第一個檔案）。
DESKTOP=$(dpkg -L "$DPKG_MAINTSCRIPT_PACKAGE" 2>/dev/null | grep '\.desktop$' | head -n 1)
[ -n "$DESKTOP" ] || exit 0
BIN=$(sed -n 's/^Exec=\([^ ]*\).*/\1/p' "$DESKTOP" | head -n 1)
[ -n "$BIN" ] || exit 0
case "$BIN" in
  /*) ;;
  *) BIN="/usr/bin/$BIN" ;;
esac

update-alternatives --install /usr/bin/x-terminal-emulator x-terminal-emulator "$BIN" 10
