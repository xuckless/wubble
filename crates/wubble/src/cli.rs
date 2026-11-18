//! Command-line arguments, parsed declaratively into one [`Command`].

use std::num::NonZeroU32;

use clap::{CommandFactory, Parser, error::ErrorKind};

#[derive(Parser, Debug)]
#[command(name = "wubble", about = "The wubble browser.")]
struct Cli {
    /// Page to open (`example.com` means `https://example.com`); the bundled
    /// page if none.
    #[arg(value_name = "URL", conflicts_with = "smoke")]
    page: Option<String>,
    /// Run a self-exiting check instead of the browser: `boot` (CEF starts and
    /// stops, no window), `frames=N` (show N zero-copy frames, then exit) or
    /// `nav` (navigate between two bundled pages, then exit).
    #[arg(long, value_name = "CHECK", value_parser = parse_smoke)]
    smoke: Option<SmokeArg>,
    /// Page for `--smoke frames=N` instead of the bundled test page.
    #[arg(long, requires = "smoke")]
    url: Option<String>,
}

/// The `--smoke` value alone; `--url` is joined in by `Command::try_from`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SmokeArg {
    Boot,
    Frames(NonZeroU32),
    Nav,
}

fn parse_smoke(value: &str) -> Result<SmokeArg, String> {
    match value.split_once('=') {
        None if value == "boot" => Ok(SmokeArg::Boot),
        None if value == "nav" => Ok(SmokeArg::Nav),
        Some(("frames", n)) => n
            .parse::<NonZeroU32>()
            .map(SmokeArg::Frames)
            .map_err(|e| format!("frames=N needs a whole number above 0: {e}")),
        _ => Err("expected `boot`, `frames=N` or `nav`".into()),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Smoke {
    /// Initialise CEF, print its version, shut down.
    Boot,
    /// Show `frames` accelerated frames of `url` (the bundled page if `None`).
    Frames {
        frames: NonZeroU32,
        url: Option<String>,
    },
    /// Load the bundled page, navigate to a second one, wait for its title.
    Nav,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// The browser, on this page (the bundled one if `None`).
    Run {
        url: Option<String>,
    },
    Smoke(Smoke),
}

impl TryFrom<Cli> for Command {
    type Error = clap::Error;

    fn try_from(cli: Cli) -> Result<Self, Self::Error> {
        match (cli.smoke, cli.url) {
            (None, _) => Ok(Command::Run {
                url: cli.page.as_deref().map(wubble_core::url::from_cli_arg),
            }),
            (Some(SmokeArg::Boot), None) => Ok(Command::Smoke(Smoke::Boot)),
            (Some(SmokeArg::Nav), None) => Ok(Command::Smoke(Smoke::Nav)),
            (Some(SmokeArg::Boot | SmokeArg::Nav), Some(_)) => Err(Cli::command().error(
                ErrorKind::ArgumentConflict,
                "--url only applies to --smoke frames=N",
            )),
            (Some(SmokeArg::Frames(frames)), url) => {
                Ok(Command::Smoke(Smoke::Frames { frames, url }))
            }
        }
    }
}

/// Parses `args` (including the program name), or returns clap's error.
pub fn parse<I, T>(args: I) -> Result<Command, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    Cli::try_parse_from(args).and_then(Command::try_from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(n: u32, url: Option<&str>) -> Option<Command> {
        Some(Command::Smoke(Smoke::Frames {
            frames: NonZeroU32::new(n).expect("test uses n > 0"),
            url: url.map(String::from),
        }))
    }

    fn run(url: Option<&str>) -> Option<Command> {
        Some(Command::Run {
            url: url.map(String::from),
        })
    }

    #[test]
    fn no_args_runs_the_browser() {
        assert_eq!(parse(["wubble"]).ok(), run(None));
    }

    #[test]
    fn a_positional_url_is_opened() {
        assert_eq!(
            parse(["wubble", "https://example.com/a"]).ok(),
            run(Some("https://example.com/a"))
        );
        assert_eq!(
            parse(["wubble", "example.com/p"]).ok(),
            run(Some("https://example.com/p"))
        );
    }

    #[test]
    fn smoke_nav() {
        assert_eq!(
            parse(["wubble", "--smoke", "nav"]).ok(),
            Some(Command::Smoke(Smoke::Nav))
        );
        assert!(parse(["wubble", "--smoke", "nav", "--url", "about:blank"]).is_err());
    }

    #[test]
    fn a_url_does_not_combine_with_a_smoke() {
        for smoke in ["boot", "nav", "frames=3"] {
            assert!(
                parse(["wubble", "--smoke", smoke, "example.com"]).is_err(),
                "{smoke}"
            );
        }
    }

    #[test]
    fn smoke_boot() {
        assert_eq!(
            parse(["wubble", "--smoke", "boot"]).ok(),
            Some(Command::Smoke(Smoke::Boot))
        );
    }

    #[test]
    fn smoke_frames_with_and_without_url() {
        assert_eq!(
            parse(["wubble", "--smoke", "frames=300"]).ok(),
            frames(300, None)
        );
        assert_eq!(
            parse(["wubble", "--smoke=frames=5", "--url", "file:///x.html"]).ok(),
            frames(5, Some("file:///x.html"))
        );
    }

    #[test]
    fn bad_smoke_values_are_errors() {
        for bad in [
            "nope",
            "frames",
            "frames=",
            "frames=0",
            "frames=abc",
            "frames=-1",
            "boot=1",
        ] {
            assert!(parse(["wubble", "--smoke", bad]).is_err(), "{bad}");
        }
    }

    #[test]
    fn url_needs_a_frames_smoke() {
        assert!(parse(["wubble", "--url", "about:blank"]).is_err());
        assert!(parse(["wubble", "--smoke", "boot", "--url", "about:blank"]).is_err());
    }
}
