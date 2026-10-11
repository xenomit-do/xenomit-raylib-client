#!/usr/bin/env sh
set -eu
if [ -z "${LIBCLANG_PATH:-}" ]; then
  candidate=$(find /usr/lib /usr/local/lib -name 'libclang.so*' -type f 2>/dev/null | sort -V | tail -n 1 || true)
  if [ -n "$candidate" ]; then export LIBCLANG_PATH=$(dirname "$candidate"); fi
fi
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target/linux}"
cargo build --release --bin xenomit-client
