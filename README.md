# photo-stamp-screensaver

(If you're reading this on GitHub, you're reading a mirror.)

A Windows screensaver that shows a folder of photos with **where** and **when** each was taken, the same
caption [photo-stamp](https://forgejo.terrell.ltd/dave/photo-stamp) burns in, but drawn on screen instead:
the photos are never changed. Places come from the family's place server
([photo-stamp-server](https://forgejo.terrell.ltd/dave/photo-stamp-server)) and are remembered in a hidden
file in the photo folder.

See [DESIGN.md](DESIGN.md) for the decisions behind it.

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

### Changing a caption

**Photo Stamp captions** (on the desktop and in the Start menu) changes what a photo's caption says:
the place is wrong or too vague, the photo has no GPS or date, or you'd rather it said "Grandma's
garden".

- It opens on the screensaver's folder, its photos as thumbnails. **Choose a folder...** shows
  another, and photos (or folders of them) dragged onto the window, or onto its shortcut, are shown
  instead.
- The chosen photo is shown with its caption as the screensaver draws it. Underneath, the caption's
  lines can be rewritten, one line per row, and the picture follows as you type. **Save** (or
  Ctrl+S) keeps it; **Back to automatic** goes back to the caption worked out from the photo.
  Emptying the box and saving means no caption for that photo at all. Moving to another photo with
  changes not saved asks first. Photos with a changed caption have ✎ before their name.
- A changed caption is kept in two places: **in the photo** (JPEG and PNG: an XMP property,
  `pss:Caption`, beside the photo's other metadata, which is left alone, as is its modified time),
  so it goes wherever the photo goes; and **in the folder's cache** (below), so every PC showing the
  folder has it, whatever kind of photo it is. HEIC, WebP, TIFF and other photos keep it in the
  cache only. For a photo outside any screensaver folder, only the photo itself can keep it.
- The screensaver shows the cache's caption first, then the photo's, then the automatic one.
  Photos photo-stamp already stamped can't be changed (their caption is part of the picture).

### The cache in the photo folder

What the screensaver learns is kept beside the photos in a hidden `.photo-stamp-screensaver.sqlite`: each photo's
date and position, the server's answers, and captions changed by hand. Every PC showing the folder shares it, so a spot is looked up once
for the whole family. The photos themselves are never changed. If the folder can't be written (read-only, or the
file is locked), a per-PC copy in `%LOCALAPPDATA%\photo-stamp-screensaver\` is used instead. Deleting the file is
harmless; it fills in again (captions changed by hand stay in JPEG and PNG photos, but other photos lose them).

Where the slideshow got to (shuffled photos already shown, or the last one in order) is kept per PC in
`%LOCALAPPDATA%\photo-stamp-screensaver\state.sqlite`, with a log, `screensaver.log`, beside it.

## Install

Run `photo-stamp-screensaver-setup.exe`. Windows asks for permission, because screensavers are installed for
everyone on the PC (into `System32`, the only place Screen Saver Settings lists them from). Setup:

- copies `photo-stamp-screensaver.scr` into `C:\Windows\System32`, and the caption editor
  (`photo-stamp-captions.exe`) and the towns database into `C:\Program Files\Photo Stamp Screensaver`;
- makes it the screensaver of whoever ran setup;
- adds **Photo Stamp screensaver settings** to the Start menu, and **Photo Stamp captions** to the Start menu
  and the desktop;
- opens Screen Saver Settings, where **Settings...** opens the screensaver's settings. "Wait N minutes" there
  sets when it starts, as for any screensaver.

Settings > Apps uninstalls it. That keeps the settings and the place caches in the photo folders. The setup isn't
code-signed, so SmartScreen asks once ("More info", then "Run anyway").

### Updates

The screensaver keeps itself up to date through the place server, as photo-stamp does: the server relays the
builds CI publishes (`photo-stamp-screensaver-windows`; photo-stamp-server's `/v1/client/latest?app=screensaver`).

- A scheduled task, **Photo Stamp Screensaver update**, runs `photo-stamp-screensaver.scr /update` as SYSTEM every
  day (at a random time between 10:00 and 14:00, or at the next chance) and 15 minutes after startup. It installs
  a newer build on the chosen channel (Settings > Updates: releases only, or every build), checked against the
  server's SHA-256, beside the running one (the old file is removed next time). It updates the caption editor
  the same way (`app=screensaver-captions`), and fetches a newer towns database when the server has one. It logs to `C:\ProgramData\Photo Stamp Screensaver\update.log`, and the
  settings dialog shows the last result.
- The task runs with administrator rights, so the server address and key it uses are kept machine-wide
  (`HKLM\Software\Photo Stamp Screensaver\Updates`, the key encrypted for the PC), where only an administrator
  can change them: nobody else can point it at another server. Setup copies the installing user's place server
  there (or photo-stamp's); after that, saving different server settings in the dialog offers to use them for
  updates too, with Windows asking for permission. A "No" isn't asked again for the same settings.
- It refuses builds from a server that doesn't know the screensaver yet (photo-stamp-server before
  `app=screensaver` would offer photo-stamp's own zip) and downloads that aren't Windows programs.
- Running the setup again also updates, by hand.

## Settings

| Setting               | Meaning                                                                         |
|-----------------------|---------------------------------------------------------------------------------|
| Folder                | The top folder of photos; its subfolders are included. A network share is fine  |
| Show each photo for   | 3 to 3600 seconds (10 by default)                                               |
| Show where and when   | Always, fading in and out, or never; and in which corner (bottom left by default) |
| Order                 | Shuffled (every photo once before any repeats), by date taken, or by folder     |
| Install (Updates)     | Releases only (the default), or every build of `main`                           |
| Address               | The place server, e.g. `https://places.example.com`. Empty: no places shown     |
| Device key            | This PC's key (`photo-stamp-server device add …`). **Test connection** checks both |

The first time the settings open on a PC that runs photo-stamp, its place server and key are filled in from
`%LOCALAPPDATA%\photo-stamp\client.json`. Each user's settings are in `HKCU\Software\Photo Stamp Screensaver`, and
the device key there is encrypted for that Windows user (DPAPI).

The `.scr` also runs by hand: `photo-stamp-screensaver.scr /s` (screensaver), `/c` (settings),
`/p <HWND>` (preview), `/w` (the slideshow in an ordinary window that input doesn't close, for trying it out).
For tests, `PSS_FOLDER` shows another folder than the settings', `PSS_DATA_DIR` moves the per-PC files and
`PSS_TOWNS` the towns database; the caption editor takes the same (it logs to `captions.log` in the data folder).

## Build

```sh
cargo test                   # Linux: everything but the drawing (EXIF, captions, caches, order, lookups)
cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings
cargo run --example info -- photos/*.jpg      # what the screensaver reads from photos
scripts/build-windows.sh     # dist/photo-stamp-screensaver.scr, photo-stamp-captions.exe, towns.sqlite and the setup
```

The checks need Rust with the `x86_64-pc-windows-gnu` target and mingw-w64 (`gcc`, `windres`). The real build
needs only Docker: `docker/Dockerfile.windows` cross-compiles libde265 and libheif as static libraries (the same
stages as photo-stamp's, so the slow part is shared) and builds the `.scr` and the caption editor with
`--features heif`. The towns
database is the newest one [dave/photo-stamp-places](https://forgejo.terrell.ltd/dave/photo-stamp-places)
publishes, downloaded with `scripts/fetch-places.sh`, which needs a Forgejo token that can read packages (`$TOKEN`
or `~/.config/forgejo/token`). `makensis` is used if it's installed, otherwise through Docker. Versions come from
git tags (`scripts/version.sh`), as in photo-stamp.

| Module                | Role                                                                    |
|-----------------------|-------------------------------------------------------------------------|
| `src/args.rs`         | How Windows starts a screensaver (`/s`, `/c`, `/p`, and `/w`)           |
| `src/settings.rs`     | The settings and their checks; reading photo-stamp's `client.json`      |
| `src/server.rs`       | Talking to the place server (`/v1/me`, `/v1/place`)                     |
| `src/places.rs`       | photo-stamp's place types (its `proto` crate) and naming from towns alone |
| `src/exif.rs`         | What the caption needs from EXIF (`kamadak-exif`), and photo-stamp's marker |
| `src/xmp.rs`          | A changed caption kept in the photo: reading and writing `pss:Caption` (`img-parts`, `quick-xml`) |
| `src/caption.rs`      | The caption's lines, with the local time at the photo's position        |
| `src/geo.rs`          | Place lookups: the server, remembered answers, the nearest town         |
| `src/cache.rs`        | The shared cache in the photo folder, and its per-PC fallback          |
| `src/playlist.rs`     | Finding the photos, the order, and where the slideshow got to          |
| `src/library.rs`      | Getting the next photo ready: bytes, metadata, caption (changed, or worked out); saving a change |
| `src/photos.rs`       | The caption editor's photos, and the top folder (and cache) each belongs to |
| `src/slide.rs`        | Framing, pan/zoom, crossfade and caption timing (arithmetic only)       |
| `src/update.rs`       | Updates: newer or not, SHA-256, swapping a running file, the towns database |
| `src/heif.rs`         | HEIC through libheif                                                    |
| `src/win/decode.rs`   | Decoding for the screen with WIC (and libheif), scaled and turned upright |
| `src/win/saver.rs`    | The screensaver window, a slideshow per monitor, drawn with Direct2D    |
| `src/win/draw.rs`     | Direct2D shared by both: photos onto the GPU, the caption's look        |
| `src/win/captions.rs` | The caption editor's window: thumbnails, preview, drag and drop         |
| `src/bin/photo-stamp-captions.rs` | The caption editor's program                                |
| `src/win/dialog.rs`   | The settings dialog (laid out in `assets/screensaver.rc`)               |
| `src/win/folder.rs`   | The folder picker                                                       |
| `src/win/store.rs`    | Settings in the registry (per user, and the updater's machine-wide), the keys encrypted |
| `src/win/updater.rs`  | The update task (`/update`), setup's `/install` and `/uninstall`, `/updates-set` |
| `installer/`          | The setup program (NSIS)                                                |
| `docker/`             | The Windows cross-build with libheif                                    |
| `assets/`             | Icon, manifest and resources                                            |
| `tests/fixtures/`     | photo-stamp's small places database, for the towns tests                |

## License

MIT. The towns database is built from [GeoNames](https://www.geonames.org) (CC BY 4.0).
