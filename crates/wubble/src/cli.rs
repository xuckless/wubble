//! Command-line arguments, parsed declaratively into one [`Command`].

use std::num::NonZeroU32;

use clap::{CommandFactory, Parser, error::ErrorKind};

#[derive(Parser, Debug)]
#[command(name = "wubble", about = "The wubble browser.")]
struct Cli {
    /// Run a self-exiting check instead of the browser: `boot` (CEF starts and
    /// stops, no window) or `frames=N` (show N zero-copy frames, then exit;
    /// on the bundled page, also check the window shows it).
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
}

fn parse_smoke(value: &str) -> Result<SmokeArg, String> {
    match value.split_once('=') {
        None if value == "boot" => Ok(SmokeArg::Boot),
        Some(("frames", n)) => n
            .parse::<NonZeroU32>()
            .map(SmokeArg::Frames)
            .map_err(|e| format!("frames=N needs a whole number above 0: {e}")),
        _ => Err("expected `boot` or `frames=N`".into()),
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Run,
    Smoke(Smoke),
}

impl TryFrom<Cli> for Command {
    type Error = clap::Error;

    fn try_from(cli: Cli) -> Result<Self, Self::Error> {
        match (cli.smoke, cli.url) {
            (None, _) => Ok(Command::Run),
            (Some(SmokeArg::Boot), None) => Ok(Command::Smoke(Smoke::Boot)),
            (Some(SmokeArg::Boot), Some(_)) => Err(Cli::command().error(
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

    #[test]
    fn no_args_runs_the_browser() {
        assert_eq!(parse(["wubble"]).ok(), Some(Command::Run));
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
