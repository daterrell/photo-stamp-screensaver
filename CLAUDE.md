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
  into a scratch folder: the screensaver writes its cache file into whatever folder it shows.

## Gotchas

- `src/places.rs` and the protocol types are copied from photo-stamp's `proto` (`/v1`); keep them in step.
- No new third-party crates without asking: the user prefers the ones photo-stamp already uses.
- `docker/Dockerfile.windows`' toolchain and codecs stages are byte-for-byte photo-stamp's, so the slow codec
  build is cached once for both. Change the screensaver's own `decoders` stage instead.
- The setup is 32-bit NSIS: `$SYSDIR` is SysWOW64. Use `$WINDIR\System32` with `${DisableX64FSRedirection}`.
- The user may have a real install: settings live in `HKCU\Software\Photo Stamp Screensaver` and the setup writes
  `HKCU\Control Panel\Desktop\SCRNSAVE.EXE`. Look before changing them in tests, and kill only PIDs you started.
- Starting a `.scr` through the shell (`Start-Process`) goes through its default verb (`"%1" /S`), so `/c` may not
  arrive; start it with `UseShellExecute = false`.
