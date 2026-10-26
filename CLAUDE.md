# photo-stamp-screensaver

A Windows screensaver showing a folder of unstamped photos with photo-stamp's where/when caption drawn on
screen; places come from photo-stamp-server and are cached in a hidden SQLite file in the photo folder.
README.md has the features and module table, DESIGN.md the decisions and open questions. Hosted on Forgejo as
`dave/photo-stamp-screensaver` (`origin`). Default branch is `main`. The users are the user's
non-technical family, as with photo-stamp.

## Build and test

- Rust is installed locally (with the `x86_64-pc-windows-gnu` target and mingw-w64); there's no dev
  container here, unlike photo-stamp.
- CI runs `cargo fmt --all --check`, `cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked`
  and `cargo clippy --target x86_64-pc-windows-gnu --all-targets --locked -- -D warnings`. Most of the code is
  `cfg(windows)`, so the Windows clippy is the one that catches it.
- `scripts/build-windows.sh` makes `dist/photo-stamp-screensaver.scr` (through Docker, with libheif) and the
  setup. A quick local `.scr` without HEIC: `cargo build --release --target x86_64-pc-windows-gnu`.
- Try the slideshow on the Windows side with `/w` (a normal window input doesn't close) and `PSS_FOLDER`,
  `PSS_DATA_DIR`, so a test touches neither the real settings' folder nor the real per-PC state. Copy photos
  into a scratch folder: the screensaver writes its cache file into whatever folder it shows, and the caption
  editor (`photo-stamp-captions.exe`, same variables) writes into the photos themselves.
- Drive the caption editor with messages sent to its own controls (`WM_SETTEXT` to the edit box, `BM_CLICK`
  to buttons, keys posted to the list), never SendKeys: that types into whichever window is in front, which
  may be the user's terminal. The managed UI Automation client sees its controls only as panes.

## Gotchas

- photo-stamp's `proto` is a git dependency on its GitHub mirror, pinned by `rev`: bump it to pick up protocol
  changes. Prefer established crates to hand-rolled code (the user asked for that in round 3).
- `docker/Dockerfile.windows`' toolchain and codecs stages are byte-for-byte photo-stamp's, so the slow codec
  build is cached once for both. Change the screensaver's own `decoders` stage instead.
- The setup is 32-bit NSIS: `$SYSDIR` is SysWOW64. Use `$WINDIR\System32` with `${DisableX64FSRedirection}`.
- The user may have a real install: settings live in `HKCU\Software\Photo Stamp Screensaver` and the setup writes
  `HKCU\Control Panel\Desktop\SCRNSAVE.EXE`. Look before changing them in tests, and kill only PIDs you started.
- Starting a `.scr` through the shell (ShellExecute, `Start-Process` without `-NoNewWindow`) runs its default verb,
  `"%1" /S`, whatever the arguments, and Task Scheduler won't start one at all ("file not found"). Start it with
  CreateProcess (`UseShellExecute = false`, NSIS `ExecWait`) or through `cmd.exe /c`, as the update task and the
  dialog's elevated `/updates-set` do.
- The user's WSL terminal runs elevated (High integrity): writing to Program Files or HKLM from it working
  proves nothing about what a normal user can do.
- Testing the updater on the real PC changes `HKLM\Software\Photo Stamp Screensaver\Updates`; `/install` (started
  directly) puts the installing user's settings back if a test clobbered them.
