#!/usr/bin/env bash
# Cross-builds reforge.dll on Linux (mingw-w64) and runs the harness under Wine.
# Needs: rustup target add x86_64-pc-windows-gnu; apt install mingw-w64 wine64.
# Release builds come from CI (MSVC); this is for development and testing.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/native"
cargo build --release --locked --target x86_64-pc-windows-gnu
out="$root/native/target/x86_64-pc-windows-gnu/release"
WINEDEBUG=-all wine "$out/reforge-harness.exe" "$out/reforge.dll"
python3 "$root/scripts/make_dist.py" "$out/reforge.dll"
