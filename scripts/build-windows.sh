#!/usr/bin/env bash
# Build the screensaver and its setup program:
#
#   dist/photo-stamp-screensaver.scr        (the program; also runnable by hand: /s, /c, /w)
#   dist/photo-stamp-captions.exe           (the caption editor)
#   dist/towns.sqlite                       (photo-stamp's towns database, for places while offline)
#   dist/photo-stamp-screensaver-setup.exe  (what people download: installer/screensaver.nsi)
#
# Needs only Docker: the .scr is cross-compiled with libheif linked in statically
# (docker/Dockerfile.windows), and makensis is used directly if it's installed. Set PSS_SCR and
# PSS_CAPTIONS to package programs that were already built, PSS_TOWNS_DB for the towns database
# (default data/towns.sqlite, downloaded with scripts/fetch-places.sh if missing).
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p dist
rm -f dist/photo-stamp-screensaver.scr dist/photo-stamp-captions.exe dist/towns.sqlite \
  dist/photo-stamp-screensaver-setup.exe

if [ -n "${PSS_SCR:-}" ]; then
  cp "$PSS_SCR" dist/photo-stamp-screensaver.scr
  cp "${PSS_CAPTIONS:?set PSS_CAPTIONS too}" dist/photo-stamp-captions.exe
else
  echo "building photo-stamp-screensaver.scr and photo-stamp-captions.exe…"
  docker build -q --target build -t photo-stamp-screensaver-win-build -f docker/Dockerfile.windows \
    --build-arg PSS_VERSION="$(scripts/version.sh)" . >/dev/null
  container=$(docker create photo-stamp-screensaver-win-build)
  trap 'docker rm -f "$container" >/dev/null' EXIT
  release=/src/target/x86_64-pc-windows-gnu/release
  docker cp "$container:$release/photo-stamp-screensaver.exe" dist/photo-stamp-screensaver.scr
  docker cp "$container:$release/photo-stamp-captions.exe" dist/photo-stamp-captions.exe
fi

towns="${PSS_TOWNS_DB:-data/towns.sqlite}"
[ -f "$towns" ] || scripts/fetch-places.sh "$towns"
cp "$towns" dist/towns.sqlite

# Windows wants a four-number version for its file properties: 0.7.1-dev.3+g1b37dbe → 0.7.1.0.
version=$(scripts/version.sh)
numeric=$(echo "$version" | sed -E 's/^([0-9]+)\.([0-9]+)\.([0-9]+).*/\1.\2.\3.0/')
nsis_args=(-V2 -WX "-DVERSION=$version" "-DVIVERSION=$numeric" "-DSCR=../dist/photo-stamp-screensaver.scr"
  "-DCAPTIONS=../dist/photo-stamp-captions.exe" "-DTOWNS=../dist/towns.sqlite" "-DOUTFILE=../dist/photo-stamp-screensaver-setup.exe" installer/screensaver.nsi)
if command -v makensis >/dev/null; then
  makensis "${nsis_args[@]}"
else
  docker build -q -t photo-stamp-nsis - >/dev/null <<'DOCKERFILE'
FROM debian:trixie-slim
RUN apt-get update && apt-get install -y --no-install-recommends nsis && rm -rf /var/lib/apt/lists/*
DOCKERFILE
  docker run --rm --user "$(id -u):$(id -g)" -v "$PWD:/w" -w /w photo-stamp-nsis makensis "${nsis_args[@]}"
fi
size() { du -h "$1" | cut -f1; }
echo "built $version: dist/photo-stamp-screensaver.scr ($(size dist/photo-stamp-screensaver.scr)),"\
  "dist/photo-stamp-captions.exe ($(size dist/photo-stamp-captions.exe)) and"\
  "dist/photo-stamp-screensaver-setup.exe ($(size dist/photo-stamp-screensaver-setup.exe))"
