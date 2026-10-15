#!/usr/bin/env bash
# Build (if the source changed) and run WINCI from this checkout.
#
#   ./run.sh            window on a desktop, terminal UI over SSH / no display
#   ./run.sh --tui      force the terminal UI
#   ./run.sh --gui      force the window
#
# Always the release build: a debug build encrypts at a few tens of MB/s, so
# transfers would crawl. The first build takes a few minutes (full LTO);
# after that Cargo only rebuilds when the source changes.
set -euo pipefail

APP="$(cd "$(dirname "$0")/app" && pwd)"

# ssh sessions often skip the login profile that puts rustup on PATH
if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
fi
if ! command -v cargo >/dev/null 2>&1; then
    echo "run.sh: Rust isn't installed — get it from https://rustup.rs, then run this again" >&2
    exit 1
fi
if ! command -v cc >/dev/null 2>&1; then
    echo "run.sh: no C toolchain — install one first (Debian/Ubuntu: sudo apt install build-essential)" >&2
    exit 1
fi

cargo build --release --quiet --manifest-path "$APP/Cargo.toml"
exec "$APP/target/release/winci" "$@"
