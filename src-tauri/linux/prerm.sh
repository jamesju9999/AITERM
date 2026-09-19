#!/bin/sh
# 移除套件時取消註冊。升級（upgrade）不能取消，否則使用者選的終端機會被重設。
set -e

case "$1" in
  remove|deconfigure) ;;
  *) exit 0 ;;
esac

DESKTOP=$(dpkg -L "$DPKG_MAINTSCRIPT_PACKAGE" 2>/dev/null | grep '\.desktop$' | head -n 1)
[ -n "$DESKTOP" ] || exit 0
BIN=$(sed -n 's/^Exec=\([^ ]*\).*/\1/p' "$DESKTOP" | head -n 1)
[ -n "$BIN" ] || exit 0
case "$BIN" in
  /*) ;;
  *) BIN="/usr/bin/$BIN" ;;
esac

update-alternatives --remove x-terminal-emulator "$BIN"
