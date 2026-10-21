//! How Windows starts a screensaver: `/s` to run, `/c[:HWND]` for its settings, `/p HWND` for the
//! little preview in Screen Saver Settings. Windows mixes `/c:123`, `/c 123`, `-c` and upper case.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Fullscreen slideshow, until there's input.
    Run,
    /// The settings dialog, owned by `parent` (Screen Saver Settings) when given.
    Settings { parent: Option<isize> },
    /// Draw inside the preview window `parent`.
    Preview { parent: isize },
    /// The slideshow in an ordinary window that input doesn't close (`/w`): for trying it out.
    Window,
}

/// The mode from the arguments after the program name. No arguments (double-clicking the .scr in
/// Explorer opens it with `/s`, but "Configure" passes nothing) means settings.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Mode {
    let mut args = args.into_iter();
    let Some(first) = args.next() else { return Mode::Settings { parent: None } };
    let first = first.trim().trim_start_matches(['/', '-']).to_ascii_lowercase();
    let (flag, inline) = match first.split_once(':') {
        Some((flag, hwnd)) => (flag.to_owned(), Some(hwnd.to_owned())),
        None => (first, None),
    };
    let hwnd =
        inline.or_else(|| args.next()).and_then(|s| s.trim().parse::<isize>().ok()).filter(|&h| h != 0);
    match flag.as_str() {
        "s" => Mode::Run,
        "w" => Mode::Window,
        "p" | "l" => match hwnd {
            Some(parent) => Mode::Preview { parent },
            None => Mode::Settings { parent: None },
        },
        _ => Mode::Settings { parent: hwnd },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Mode {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn windows_variants() {
        assert_eq!(p(&[]), Mode::Settings { parent: None });
        assert_eq!(p(&["/s"]), Mode::Run);
        assert_eq!(p(&["/S"]), Mode::Run);
        assert_eq!(p(&["-s"]), Mode::Run);
        assert_eq!(p(&["/c"]), Mode::Settings { parent: None });
        assert_eq!(p(&["/c:4242"]), Mode::Settings { parent: Some(4242) });
        assert_eq!(p(&["/C", "4242"]), Mode::Settings { parent: Some(4242) });
        assert_eq!(p(&["/p", "1234"]), Mode::Preview { parent: 1234 });
        assert_eq!(p(&["/P:1234"]), Mode::Preview { parent: 1234 });
        assert_eq!(p(&["/w"]), Mode::Window);
    }

    #[test]
    fn nonsense_opens_settings() {
        assert_eq!(p(&["/p"]), Mode::Settings { parent: None });
        assert_eq!(p(&["/p", "zero"]), Mode::Settings { parent: None });
        assert_eq!(p(&["/x"]), Mode::Settings { parent: None });
        assert_eq!(p(&["/c:0"]), Mode::Settings { parent: None });
    }
}
