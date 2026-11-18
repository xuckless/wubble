//! Command-line arguments, parsed declaratively into one [`Command`].

use clap::{Parser, ValueEnum};

#[derive(Parser, Debug)]
#[command(name = "wubble", about = "The wubble browser.")]
struct Cli {
    /// Run a self-exiting check instead of the browser.
    #[arg(long, value_enum)]
    smoke: Option<Smoke>,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Smoke {
    /// Initialise CEF, print its version, shut down.
    Boot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Run,
    Smoke(Smoke),
}

impl From<Cli> for Command {
    fn from(cli: Cli) -> Self {
        match cli.smoke {
            Some(smoke) => Command::Smoke(smoke),
            None => Command::Run,
        }
    }
}

/// Parses `args` (including the program name), or returns clap's error.
pub fn parse<I, T>(args: I) -> Result<Command, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    Cli::try_parse_from(args).map(Command::from)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn unknown_smoke_is_an_error() {
        assert!(parse(["wubble", "--smoke", "nope"]).is_err());
    }
}
