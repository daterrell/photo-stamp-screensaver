# photo-stamp-screensaver: design

A Windows screensaver (`.scr`) that shows a slideshow from a folder of unstamped photos, with where and when each
was taken shown as an overlay, the same caption photo-stamp would burn in. Photos are never modified. Place
answers come from the existing photo-stamp-server (`/v1`, `Authorization: Bearer <device key>`) and are cached in
a hidden SQLite file in the photo folder.

Status: rounds 1 (setup, settings) and 2 (the slideshow) built; auto-update is next.

## Round 1 scope

1. **Installer** (NSIS, built on Linux like photo-stamp's).
2. **Settings dialog** (`.scr /c`): photo folder, seconds per photo, order, the caption (always / fading in and out /
   never, and its corner), place server URL and device key (with a *Test connection* button that calls
   `GET /v1/me`).
3. Round 2: the slideshow (`/s`, `/p`, and `/w` for trying it in a window).

## Screensaver contract

Windows runs the `.scr` with:

| Args              | Meaning                                                      |
|-------------------|--------------------------------------------------------------|
| `/s`              | Run fullscreen on every monitor; exit on input               |
| `/c` or `/c:HWND` | Settings dialog, modal to HWND                               |
| `/p HWND`         | Draw into the little preview monitor in Screen Saver Settings |
| none              | Same as `/c`                                                 |

## Decisions

| Topic | Choice | Notes |
|-------|--------|-------|
| Language | Rust, 2024 edition | Reuses `photo-stamp-proto` (protocol and `Place` types) and photo-stamp's mingw cross-build |
| Install scope | Admin, into `System32` | Only screensavers in `System32` appear in the Screen Saver Settings list. Setup also sets it as the active screensaver |
| Settings storage | `HKCU\Software\Photo Stamp Screensaver`; the key encrypted with DPAPI | Per user, nothing in the photo folder |
| Credentials | Its own settings | The first time, the URL and key are pre-filled from photo-stamp's `client.json`; a separate device key is suggested |
| Cache | `<top>\.photo-stamp-screensaver.sqlite`, hidden attribute, rollback journal (WAL breaks on shares), 5 s busy timeout | Photos may be local, on a NAS shared by several PCs, or in a synced folder: falls back to `%LOCALAPPDATA%\photo-stamp-screensaver\` if the folder is read-only or the db is locked |
| Cache key | photos: relative path + size + mtime → metadata; answers: position to ~11 m (photo-stamp's key) → place, `data_version`, `rev` | Answers from an older server `data_version`/`rev` are asked again |
| Metadata | Own small EXIF reader (no exiftool, no new crates) | Spawning exiftool per photo is too slow; HEIC's EXIF comes from libheif |
| Formats | JPEG, PNG, TIFF, WebP, BMP, GIF through WIC; HEIC through a bundled libheif (decode-only, ~4 MB) | Videos and RAW skipped; hidden, dot, recycle-bin and NAS thumbnail folders skipped |
| Already-stamped photos | Shown without an overlay | Detected by the `photo-stamp` XMP keyword |
| Caption | A setting: always, fading in and out, or never; and the corner | The photo-stamp look: white, outlined, shadowed |
| Order | A setting: shuffled (no repeats until all have shown, position kept across runs), by date taken, or by folder | |
| Transitions | Crossfade with a slow pan/zoom (Ken Burns) | GPU-drawn (Direct2D) so it stays smooth on 4K |
| Updates | Automatic, through the place server like photo-stamp | See below |
| Publishing | As photo-stamp: package registry for main (beta), GitHub Release for v* tags | Set up: secrets, and the push mirror to github.com/daterrell/photo-stamp-screensaver |
| Monitors | A different photo on each, changes staggered | One window over all monitors, one Direct2D target, paced by `DwmFlush` |
| Framing | Photos keeping ≥70% when cropped to the screen fill it; others whole over a blurred copy | |
| Per-PC state | Shuffle progress and position in `%LOCALAPPDATA%`, not the shared folder | The preview doesn't use it up |
| Timing | Seconds per photo only | Windows' own "Wait N minutes" decides when it starts |
| Lookups | Done for the *next* photo while the current one shows | So the next photo is never held up by the server |
| Server unreachable | An older answer, else the nearest town from photo-stamp's towns.sqlite (installed in Program Files); asked again next time | Fallbacks aren't cached; after a failure the server rests 60 s |

## Auto-update (later round)

- **Server:** photo-stamp-server relays only `photo-stamp-windows` today. It needs an app parameter (e.g.
  `GET /v1/client/latest?app=screensaver&channel=…`) and a second package, `photo-stamp-screensaver-windows`.
  That change is in the photo-stamp-server repo, and the protocol copy in photo-stamp too.
- **Client:** the `.scr` is in System32, so an update needs admin rights. The plan is for setup to register a
  scheduled task that runs as SYSTEM daily: it checks the server with a device key stored machine-wide, checks
  the SHA-256, and swaps the file when the screensaver isn't running. When a user saves settings, the device key is
  also stored machine-wide for that task: readable only by administrators and SYSTEM (registry ACL), encrypted
  with DPAPI's machine scope. The last one saved wins.
