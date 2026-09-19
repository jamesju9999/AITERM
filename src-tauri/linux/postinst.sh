#!/bin/sh
# 把 AITerm 登記成 x-terminal-emulator 的候選。priority 40 低於多數發行版預設，
# 不會搶走使用者現有的選擇；要切換用 `update-alternatives --config x-terminal-emulator`。
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

update-alternatives --install /usr/bin/x-terminal-emulator x-terminal-emulator "$BIN" 40
