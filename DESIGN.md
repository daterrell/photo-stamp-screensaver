# photo-stamp-screensaver: design

A Windows screensaver (`.scr`) that shows a slideshow from a folder of unstamped photos, with where and when each
was taken shown as an overlay, the same caption photo-stamp would burn in. Photos are never modified. Place
answers come from the existing photo-stamp-server (`/v1`, `Authorization: Bearer <device key>`) and are cached in
a hidden SQLite file in the photo folder.

Status: rounds 1 (setup, settings), 2 (the slideshow), 3 (auto-update) and 4 (the caption editor) built.

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
| Metadata | `kamadak-exif` (no exiftool) | Spawning exiftool per photo is too slow; it reads HEIF's EXIF too |
| Formats | JPEG, PNG, TIFF, WebP, BMP, GIF through WIC; HEIC through a bundled libheif (decode-only, ~4 MB) | Videos and RAW skipped; hidden, dot, recycle-bin and NAS thumbnail folders skipped |
| Already-stamped photos | Shown without an overlay | Detected by the `photo-stamp` XMP keyword |
| Caption | A setting: always, fading in and out, or never; and the corner | The photo-stamp look: white, outlined, shadowed |
| Order | A setting: shuffled (no repeats until all have shown, position kept across runs), by date taken, or by folder | |
| Transitions | Crossfade with a slow pan/zoom (Ken Burns) | GPU-drawn (Direct2D) so it stays smooth on 4K |
| Updates | Automatic, through the place server like photo-stamp; channel setting, releases by default; towns database too | See below |
| Publishing | As photo-stamp: package registry for main (beta), GitHub Release for v* tags | Set up: secrets, and the push mirror to github.com/daterrell/photo-stamp-screensaver |
| Monitors | A different photo on each, changes staggered | One window over all monitors, one Direct2D target, paced by `DwmFlush` |
| Framing | Photos keeping ≥70% when cropped to the screen fill it; others whole over a blurred copy | |
| Per-PC state | Shuffle progress and position in `%LOCALAPPDATA%`, not the shared folder | The preview doesn't use it up |
| Timing | Seconds per photo only | Windows' own "Wait N minutes" decides when it starts |
| Lookups | Done for the *next* photo while the current one shows | So the next photo is never held up by the server |
| Server unreachable | An older answer, else the nearest town from photo-stamp's towns.sqlite (installed in Program Files); asked again next time | Fallbacks aren't cached; after a failure the server rests 60 s |

## Auto-update (round 3)

- **Server:** photo-stamp-server's `/v1/client/latest` and `/v1/client/download` take `app=screensaver`, serving
  the `.scr` from `photo-stamp-screensaver-windows` (dave/photo-stamp-server#3; protocol docs in photo-stamp#17).
- **Client:** setup registers a scheduled task that runs `photo-stamp-screensaver.scr /update` as SYSTEM, daily and
  after startup, through `cmd.exe` (Task Scheduler won't start a `.scr` itself). It installs a newer build on the
  chosen channel, checked against the server's SHA-256, by renaming the running file aside, and refreshes the
  towns database when the server's is newer.
- **Trust:** the task's server address and key are machine-wide (HKLM), writable only by administrators: setup
  copies the installing user's (or photo-stamp's), and the settings dialog offers an elevated `/updates-set` when
  saved settings differ. Builds aren't signed; the server is trusted, and only an administrator chooses it.
  Builds from a server that ignores `app` (offering photo-stamp's zip) and non-programs are refused.

## The caption editor (round 4)

| Topic | Choice | Notes |
|-------|--------|-------|
| Form | A separate small program, `photo-stamp-captions.exe` in Program Files, with Start menu and desktop shortcuts | Plain Win32 like the settings dialog; same crate, a second binary. Drag and drop on the window or the shortcut |
| What's edited | The caption's lines, as free text, one line a row | As photo-stamp's Edit stamp. Empty means no caption |
| Where it's kept | Both: in the photo (XMP `pss:Caption`, an `rdf:Seq` of lines, namespace `https://forgejo.terrell.ltd/dave/photo-stamp-screensaver/ns/1.0/`) and in the folder's cache (`edit` table, by relative path) | The photo's copy travels with it; the cache's covers every format and every PC sharing the folder |
| Writing photos | JPEG (APP1 XMP) and PNG (iTXt) through `img-parts`; the rest of the XMP packet kept as it was (`quick-xml`) | HEIC, WebP, TIFF: cache only. Written beside, read back and compared, swapped in with `ReplaceFileW`, modified time kept. Going back to automatic restores the file byte for byte |
| Precedence | Cache, then the photo's, then automatic; photo-stamp-stamped photos never get one | The cache's photo rows are refreshed after a write, since the kept modified time could hide a same-length change |
| Browsing | Thumbnails from Windows (`IShellItemImageFactory`), else decoded (HEIC); a virtual list, 400 kept | A folder of tens of thousands opens at once |
| Updates | The update task updates it too: `app=screensaver-captions`, published as `photo-stamp-captions-<v>.exe` in the screensaver's package | Its installed version is kept machine-wide (`CaptionsVersion`), so a failed update is tried again. Needs photo-stamp-server to know the app |
