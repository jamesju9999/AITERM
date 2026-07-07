#!/bin/bash
# Drop-in `cargo` replacement for Tauri's `build.runner` config (macOS dev only).
#
# `tauri dev` runs `cargo run ...` directly, bypassing the bundler's
# code-signing step entirely — so the resulting binary is ad-hoc signed
# with an identity tied to that exact build's bytes, and macOS Keychain
# re-prompts for "永遠允許" on every rebuild that actually changes the binary.
#
# This script builds first, code-signs the result with a stable identity
# (if $APPLE_SIGNING_IDENTITY is set), then runs it — so Keychain sees the
# same signing identity across rebuilds and remembers the grant.
#
# Passthrough behavior (safe no-op for anyone who hasn't opted in):
#   - Any subcommand other than `run` (e.g. `cargo build --release` during
#     `tauri build`) goes straight to real cargo, untouched — `tauri build`
#     keeps using its own bundler-driven signing pipeline.
#   - If $APPLE_SIGNING_IDENTITY isn't set, the binary just runs unsigned
#     (ad-hoc) exactly as it would without this script.
set -e

REAL_CARGO=$(command -v cargo)

if [ "$1" != "run" ]; then
  exec "$REAL_CARGO" "$@"
fi
shift

# Split remaining args at the first bare "--": everything before it are
# cargo args, everything after it are the program's own runtime args —
# same convention `cargo run` itself uses.
cargo_args=()
prog_args=()
in_prog_args=false
for a in "$@"; do
  if [ "$in_prog_args" = true ]; then
    prog_args+=("$a")
  elif [ "$a" = "--" ]; then
    in_prog_args=true
  else
    cargo_args+=("$a")
  fi
done

"$REAL_CARGO" build "${cargo_args[@]}"

BIN_PATH="target/debug/app"

if [ -n "$APPLE_SIGNING_IDENTITY" ] && [ -x "$BIN_PATH" ]; then
  codesign --force --sign "$APPLE_SIGNING_IDENTITY" --timestamp=none "$BIN_PATH"
fi

exec "$BIN_PATH" "${prog_args[@]}"
