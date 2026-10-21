# photo-stamp-screensaver

(If you're reading this on GitHub, you're reading a mirror.)

A Windows screensaver that shows a folder of photos with **where** and **when** each was taken, the same
caption [photo-stamp](https://forgejo.terrell.ltd/dave/photo-stamp) burns in, but drawn on screen instead:
the photos are never changed. Places come from the family's place server
([photo-stamp-server](https://forgejo.terrell.ltd/dave/photo-stamp-server)) and are remembered in a hidden
file in the photo folder.

**Status:** round 1 has the setup program and the settings. The screensaver itself is a placeholder for now: a
black screen naming the photo folder. See [DESIGN.md](DESIGN.md) for the plan.

## Install

Run `photo-stamp-screensaver-setup.exe`. Windows asks for permission, because screensavers are installed for
everyone on the PC (into `System32`, the only place Screen Saver Settings lists them from). Setup:

- copies `photo-stamp-screensaver.scr` into `C:\Windows\System32`;
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
| Address               | The place server, e.g. `https://places.example.com`. Empty: no places shown     |
| Device key            | This PC's key (`photo-stamp-server device add …`). **Test connection** checks both |

The first time the settings open on a PC that runs photo-stamp, its place server and key are filled in from
`%LOCALAPPDATA%\photo-stamp\client.json`. Each user's settings are in `HKCU\Software\Photo Stamp Screensaver`, and
the device key there is encrypted for that Windows user (DPAPI).

The `.scr` also runs by hand: `photo-stamp-screensaver.scr /s` (screensaver), `/c` (settings),
`/p <HWND>` (preview).

## Build

```sh
cargo test                   # Linux: the platform-independent parts
cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings
scripts/build-windows.sh     # dist/photo-stamp-screensaver.scr and dist/photo-stamp-screensaver-setup.exe
```

Needs Rust with the `x86_64-pc-windows-gnu` target and mingw-w64 (`gcc`, `windres`). `makensis` is used if it's
installed, otherwise through Docker. Versions come from git tags (`scripts/version.sh`), as in photo-stamp.

| Module                | Role                                                                    |
|-----------------------|-------------------------------------------------------------------------|
| `src/args.rs`         | How Windows starts a screensaver (`/s`, `/c`, `/p`)                     |
| `src/settings.rs`     | The settings and their checks; reading photo-stamp's `client.json`      |
| `src/server.rs`       | Talking to the place server (`/v1/me` for now)                          |
| `src/win/dialog.rs`   | The settings dialog (laid out in `assets/screensaver.rc`)               |
| `src/win/folder.rs`   | The folder picker                                                       |
| `src/win/store.rs`    | Settings in the registry, the device key encrypted                      |
| `src/win/saver.rs`    | The fullscreen and preview windows                                      |
| `installer/`          | The setup program (NSIS)                                                |
| `assets/`             | Icon, manifest and resources                                            |

## License

MIT.
