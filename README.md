# photo-stamp-screensaver

(If you're reading this on GitHub, you're reading a mirror.)

A Windows screensaver that shows a folder of photos with **where** and **when** each was taken, the same
caption [photo-stamp](https://forgejo.terrell.ltd/dave/photo-stamp) burns in, but drawn on screen instead:
the photos are never changed. Places come from the family's place server
([photo-stamp-server](https://forgejo.terrell.ltd/dave/photo-stamp-server)) and are remembered in a hidden
file in the photo folder.

See [DESIGN.md](DESIGN.md) for the decisions behind it and what comes next (auto-update).

**Downloads:** `photo-stamp-screensaver-setup-<version>.exe` on the
[Releases](https://github.com/daterrell/photo-stamp-screensaver/releases) page. Every build of `main` (on
[Forgejo](https://forgejo.terrell.ltd/dave/photo-stamp-screensaver/actions)) is also in Forgejo's package
registry as `photo-stamp-screensaver-windows` (the newest 10 betas are kept). To release, tag a commit on `main`:
`git tag v0.1.0 && git push origin v0.1.0`.

## What it shows

- **Every photo in the folder and its subfolders:** JPEG, PNG, TIFF, WebP, BMP, GIF, and HEIC (iPhone) without
  Windows' paid HEVC extension (libheif is built in). Hidden files and folders, recycle bins and NAS thumbnail
  folders (`@eaDir`, `#recycle`) are skipped; so are videos and RAW files.
- **Each monitor its own slideshow**, changing one after another. Photos close to the screen's shape fill it;
  others (portraits on a landscape screen) show whole over a blurred, dimmed copy of themselves. Each photo
  slowly pans and zooms, and dissolves into the next.
- **The caption** is photo-stamp's: the place on up to three lines, then the local time *where the photo was
  taken* (a camera left on home time is corrected from its UTC offset or the GPS time), in white with a dark
  outline. A photo without GPS shows only its date; without either, nothing. Photos photo-stamp already
  stamped show no caption, so it isn't there twice.
- **Places, the first time a spot comes up,** are asked of the place server while the photo before it is on
  screen, so nobody waits. While the server can't be reached, an older answer or the nearest town (from
  photo-stamp's towns database, installed with the screensaver) is shown, and the server is asked again next
  time.

### The cache in the photo folder

What the screensaver learns is kept beside the photos in a hidden `.photo-stamp-screensaver.sqlite`: each photo's
date and position, and the server's answers. Every PC showing the folder shares it, so a spot is looked up once
for the whole family. The photos themselves are never changed. If the folder can't be written (read-only, or the
file is locked), a per-PC copy in `%LOCALAPPDATA%\photo-stamp-screensaver\` is used instead. Deleting the file is
harmless; it fills in again.

Where the slideshow got to (shuffled photos already shown, or the last one in order) is kept per PC in
`%LOCALAPPDATA%\photo-stamp-screensaver\state.sqlite`, with a log, `screensaver.log`, beside it.

## Install

Run `photo-stamp-screensaver-setup.exe`. Windows asks for permission, because screensavers are installed for
everyone on the PC (into `System32`, the only place Screen Saver Settings lists them from). Setup:

- copies `photo-stamp-screensaver.scr` into `C:\Windows\System32`, and the towns database into
  `C:\Program Files\Photo Stamp Screensaver`;
- makes it the screensaver of whoever ran setup;
- adds **Photo Stamp screensaver settings** to the Start menu;
- opens Screen Saver Settings, where **Settings...** opens the screensaver's settings. "Wait N minutes" there
  sets when it starts, as for any screensaver.

Settings > Apps uninstalls it. That keeps the settings and the place caches in the photo folders. Run the setup
again to update. The setup isn't code-signed, so SmartScreen asks once ("More info", then "Run anyway").

## Settings

| Setting               | Meaning                                                                         |
|-----------------------|---------------------------------------------------------------------------------|
| Folder                | The top folder of photos; its subfolders are included. A network share is fine  |
| Show each photo for   | 3 to 3600 seconds (10 by default)                                               |
| Show where and when   | Always, fading in and out, or never; and in which corner (bottom left by default) |
| Order                 | Shuffled (every photo once before any repeats), by date taken, or by folder     |
| Address               | The place server, e.g. `https://places.example.com`. Empty: no places shown     |
| Device key            | This PC's key (`photo-stamp-server device add …`). **Test connection** checks both |

The first time the settings open on a PC that runs photo-stamp, its place server and key are filled in from
`%LOCALAPPDATA%\photo-stamp\client.json`. Each user's settings are in `HKCU\Software\Photo Stamp Screensaver`, and
the device key there is encrypted for that Windows user (DPAPI).

The `.scr` also runs by hand: `photo-stamp-screensaver.scr /s` (screensaver), `/c` (settings),
`/p <HWND>` (preview), `/w` (the slideshow in an ordinary window that input doesn't close, for trying it out).
For tests, `PSS_FOLDER` shows another folder than the settings', `PSS_DATA_DIR` moves the per-PC files and
`PSS_TOWNS` the towns database.

## Build

```sh
cargo test                   # Linux: everything but the drawing (EXIF, captions, caches, order, lookups)
cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings
cargo run --example info -- photos/*.jpg      # what the screensaver reads from photos
scripts/build-windows.sh     # dist/photo-stamp-screensaver.scr, towns.sqlite and the setup
```

The checks need Rust with the `x86_64-pc-windows-gnu` target and mingw-w64 (`gcc`, `windres`). The real build
needs only Docker: `docker/Dockerfile.windows` cross-compiles libde265 and libheif as static libraries (the same
stages as photo-stamp's, so the slow part is shared) and builds the `.scr` with `--features heif`. The towns
database is the newest one [dave/photo-stamp-places](https://forgejo.terrell.ltd/dave/photo-stamp-places)
publishes, downloaded with `scripts/fetch-places.sh`, which needs a Forgejo token that can read packages (`$TOKEN`
or `~/.config/forgejo/token`). `makensis` is used if it's installed, otherwise through Docker. Versions come from
git tags (`scripts/version.sh`), as in photo-stamp.

| Module                | Role                                                                    |
|-----------------------|-------------------------------------------------------------------------|
| `src/args.rs`         | How Windows starts a screensaver (`/s`, `/c`, `/p`, and `/w`)           |
| `src/settings.rs`     | The settings and their checks; reading photo-stamp's `client.json`      |
| `src/server.rs`       | Talking to the place server (`/v1/me`, `/v1/place`)                     |
| `src/places.rs`       | Place types and the towns database, copied from photo-stamp's `proto`   |
| `src/exif.rs`         | A small EXIF reader: dates, offset, GPS, orientation, photo-stamp's marker |
| `src/caption.rs`      | The caption's lines, with the local time at the photo's position        |
| `src/geo.rs`          | Place lookups: the server, remembered answers, the nearest town         |
| `src/cache.rs`        | The shared cache in the photo folder, and its per-PC fallback          |
| `src/playlist.rs`     | Finding the photos, the order, and where the slideshow got to          |
| `src/library.rs`      | Getting the next photo ready: bytes, metadata, caption                  |
| `src/slide.rs`        | Framing, pan/zoom, crossfade and caption timing (arithmetic only)       |
| `src/heif.rs`         | HEIC through libheif                                                    |
| `src/win/decode.rs`   | Decoding for the screen with WIC (and libheif), scaled and turned upright |
| `src/win/saver.rs`    | The screensaver window, a slideshow per monitor, drawn with Direct2D    |
| `src/win/dialog.rs`   | The settings dialog (laid out in `assets/screensaver.rc`)               |
| `src/win/folder.rs`   | The folder picker                                                       |
| `src/win/store.rs`    | Settings in the registry, the device key encrypted                      |
| `installer/`          | The setup program (NSIS)                                                |
| `docker/`             | The Windows cross-build with libheif                                    |
| `assets/`             | Icon, manifest and resources                                            |
| `tests/fixtures/`     | photo-stamp's small places database, for the towns tests                |

## License

MIT. The towns database is built from [GeoNames](https://www.geonames.org) (CC BY 4.0).
