#!/usr/bin/env bash
# Build the screensaver and its setup program:
#
#   dist/photo-stamp-screensaver.scr        (the program; also runnable by hand: /s, /c)
#   dist/photo-stamp-screensaver-setup.exe  (what people download: installer/screensaver.nsi)
#
# Needs Rust with the x86_64-pc-windows-gnu target and mingw-w64 (gcc and windres), and makensis
# (Docker is used for it when it isn't installed). Set PSS_SCR to package a .scr that was already
# built.
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p dist
rm -f dist/photo-stamp-screensaver.scr dist/photo-stamp-screensaver-setup.exe

if [ -n "${PSS_SCR:-}" ]; then
  cp "$PSS_SCR" dist/photo-stamp-screensaver.scr
else
  cargo build --release --locked --target x86_64-pc-windows-gnu
  cp target/x86_64-pc-windows-gnu/release/photo-stamp-screensaver.exe dist/photo-stamp-screensaver.scr
fi

# Windows wants a four-number version for its file properties: 0.7.1-dev.3+g1b37dbe → 0.7.1.0.
version=$(scripts/version.sh)
numeric=$(echo "$version" | sed -E 's/^([0-9]+)\.([0-9]+)\.([0-9]+).*/\1.\2.\3.0/')
nsis_args=(-V2 -WX "-DVERSION=$version" "-DVIVERSION=$numeric" "-DSCR=../dist/photo-stamp-screensaver.scr"
  "-DOUTFILE=../dist/photo-stamp-screensaver-setup.exe" installer/screensaver.nsi)
if command -v makensis >/dev/null; then
  makensis "${nsis_args[@]}"
else
  docker build -q -t photo-stamp-nsis - >/dev/null <<'DOCKERFILE'
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends nsis && rm -rf /var/lib/apt/lists/*
DOCKERFILE
  docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/w" -w /w photo-stamp-nsis makensis "${nsis_args[@]}"
fi
echo "built $version: dist/photo-stamp-screensaver.scr ($(du -h dist/photo-stamp-screensaver.scr | cut -f1)) and dist/photo-stamp-screensaver-setup.exe ($(du -h dist/photo-stamp-screensaver-setup.exe | cut -f1))"
