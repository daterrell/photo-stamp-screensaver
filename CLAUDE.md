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
- `scripts/build-windows.sh` makes `dist/photo-stamp-screensaver.scr` and the setup.

## Gotchas

- The protocol types are a subset of photo-stamp's `proto/src/api.rs` (`/v1`); keep them in step.
- The setup is 32-bit NSIS: `$SYSDIR` is SysWOW64. Use `$WINDIR\System32` with `${DisableX64FSRedirection}`.
- The user may have a real install: settings live in `HKCU\Software\Photo Stamp Screensaver` and the setup writes
  `HKCU\Control Panel\Desktop\SCRNSAVE.EXE`. Look before changing them in tests, and kill only PIDs you started.
- Starting a `.scr` through the shell (`Start-Process`) goes through its default verb (`"%1" /S`), so `/c` may not
  arrive; start it with `UseShellExecute = false`.
