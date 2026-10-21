#!/usr/bin/env bash
# Build the screensaver and its setup program:
#
#   dist/photo-stamp-screensaver.scr        (the program; also runnable by hand: /s, /c, /w)
#   dist/towns.sqlite                       (photo-stamp's towns database, for places while offline)
#   dist/photo-stamp-screensaver-setup.exe  (what people download: installer/screensaver.nsi)
#
# Needs only Docker: the .scr is cross-compiled with libheif linked in statically
# (docker/Dockerfile.windows), and makensis is used directly if it's installed. Set PSS_SCR to
# package a .scr that was already built, PSS_TOWNS_DB for the towns database (default
# data/towns.sqlite, downloaded with scripts/fetch-places.sh if missing).
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p dist
rm -f dist/photo-stamp-screensaver.scr dist/towns.sqlite dist/photo-stamp-screensaver-setup.exe

if [ -n "${PSS_SCR:-}" ]; then
  cp "$PSS_SCR" dist/photo-stamp-screensaver.scr
else
  echo "building photo-stamp-screensaver.scr…"
  docker build -q --target build -t photo-stamp-screensaver-win-build -f docker/Dockerfile.windows \
    --build-arg PSS_VERSION="$(scripts/version.sh)" . >/dev/null
  container=$(docker create photo-stamp-screensaver-win-build)
  trap 'docker rm -f "$container" >/dev/null' EXIT
  docker cp "$container:/src/target/x86_64-pc-windows-gnu/release/photo-stamp-screensaver.exe" \
    dist/photo-stamp-screensaver.scr
fi

towns="${PSS_TOWNS_DB:-data/towns.sqlite}"
[ -f "$towns" ] || scripts/fetch-places.sh "$towns"
cp "$towns" dist/towns.sqlite

# Windows wants a four-number version for its file properties: 0.7.1-dev.3+g1b37dbe → 0.7.1.0.
version=$(scripts/version.sh)
numeric=$(echo "$version" | sed -E 's/^([0-9]+)\.([0-9]+)\.([0-9]+).*/\1.\2.\3.0/')
nsis_args=(-V2 -WX "-DVERSION=$version" "-DVIVERSION=$numeric" "-DSCR=../dist/photo-stamp-screensaver.scr"
  "-DTOWNS=../dist/towns.sqlite" "-DOUTFILE=../dist/photo-stamp-screensaver-setup.exe" installer/screensaver.nsi)
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
