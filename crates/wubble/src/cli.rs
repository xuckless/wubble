//! Command-line arguments, parsed declaratively into one [`Command`].

use std::num::NonZeroU32;
use std::path::PathBuf;

use clap::{CommandFactory, Parser, error::ErrorKind};

pub use crate::load_run::Variant;
pub use crate::measure::Scenario;

/// `--measure`'s run time unless `--seconds` says otherwise.
const DEFAULT_SECONDS: NonZeroU32 = NonZeroU32::new(10).unwrap();

#[derive(Parser, Debug)]
#[command(name = "wubble", about = "The wubble browser.")]
struct Cli {
    /// Page to open (`example.com` means `https://example.com`); the bundled
    /// page if none.
    #[arg(value_name = "URL", conflicts_with_all = ["smoke", "measure"])]
    page: Option<String>,
    /// Run a self-exiting check instead of the browser: `boot` (CEF starts and
    /// stops, no window), `frames=N` (show N zero-copy frames, then exit; on
    /// the bundled page, also check the window shows it), `nav` (navigate between two bundled pages, then exit), `tabs` (open,
    /// switch, sleep, restore and close tabs on bundled pages, then exit), `split` (two
    /// bundled pages side by side, each checked, then unsplit and exit), `page`
    /// (a bundled page's select popup, alert, context menu, find and zoom,
    /// then exit), `block` (a bundled page's blocked requests are counted, then exit), `settings` (open and flip a setting), `pages` (open history, bookmarks, downloads and passwords on a seeded profile), `session` (open two tabs, quit, start again and check they come back), `autofill` (fill, submit and save on a loopback sign-in page) or `windows` (move a tab into a second window, check both, close it).
    /// `session-phase=1|2` is one half of `session`, run by it.
    #[arg(long, value_name = "CHECK", value_parser = parse_smoke, conflicts_with = "measure")]
    smoke: Option<SmokeArg>,
    /// Measure the frame path on a bundled page and print the numbers (hand
    /// run only): `scroll` (scripted scroll), `video` (CSS and canvas
    /// animation), `resize` (the window resizes itself), `typing` (keys and
    /// clicks, timed to the first frame that shows them), `idle` (a still page:
    /// renders, wake-ups, CPU and GPU), `startup` (start-up stages to the first
    /// page pixels), `blocker` (the content blocker, windowless) or `memory` (the
    /// process tree's memory with 0, 1, 10 and 50 tabs, then frozen and
    /// discarded). Exits by itself.
    #[arg(long, value_name = "SCENARIO", value_enum, conflicts_with = "profile")]
    measure: Option<Scenario>,
    /// How long `--measure` runs.
    #[arg(long, value_name = "N", requires = "measure", default_value_t = DEFAULT_SECONDS)]
    seconds: NonZeroU32,
    /// How long `--measure` waits after the first frame before it records, instead of the
    /// scenario's own (`idle`: long enough for the scene to have settled).
    #[arg(long, value_name = "N", requires = "measure")]
    warmup: Option<NonZeroU32>,
    /// `--measure` with CEF paced by external begin frames, one per Slint
    /// render, instead of CEF's own timer.
    #[arg(long, requires = "measure")]
    begin_frames: bool,
    /// `--measure load`: what the engine hooks requests with (default: as shipped).
    #[arg(long, value_name = "VARIANT", value_enum, requires = "measure")]
    load_variant: Option<Variant>,
    /// `--measure load`: warm loads of each page after its cold one (default 5).
    #[arg(long, value_name = "N", requires = "measure")]
    runs: Option<NonZeroU32>,
    /// `--measure load`: load this path of the loopback server instead of the standard pages
    /// (repeatable; `/spec` is the speculation page).
    #[arg(long, value_name = "PATH", requires = "measure")]
    load_page: Vec<String>,
    /// `--measure load`: after the last page, go to another and back by history.
    #[arg(long, requires = "measure")]
    load_back: bool,
    /// `--measure load`: serve the pages on this loopback port (0: any) and print each timing a
    /// page posts, for `scripts/measure-load-chromium.sh`; no window.
    #[arg(long, value_name = "PORT", requires = "measure")]
    serve_only: Option<u16>,
    /// Also write `--measure`'s samples as CSV to this file.
    #[arg(long, value_name = "PATH", requires = "measure")]
    out: Option<PathBuf>,
    /// Page for `--smoke frames=N` instead of the bundled test page.
    #[arg(long, requires = "smoke", conflicts_with = "measure")]
    url: Option<String>,
    /// Profile directory (database and CEF cache) instead of the per-user one.
    #[arg(long, value_name = "DIR")]
    profile: Option<PathBuf>,
    /// Where downloads go instead of the OS downloads folder.
    #[arg(long, value_name = "DIR", conflicts_with = "smoke")]
    downloads: Option<PathBuf>,
    /// Hand run only: write the window as a PNG when the smoke ends (`settings` also seeds
    /// three Spaces and a few tabs at the design's 1360x880, to compare with the design).
    #[arg(long, value_name = "PNG", requires = "smoke")]
    shot: Option<PathBuf>,
    /// Hand run only, with `--smoke gallery`: the folder the gallery writes one numbered PNG
    /// per shell surface into.
    #[arg(long, value_name = "DIR", requires = "smoke", conflicts_with = "shot")]
    shot_dir: Option<PathBuf>,
    /// Hand run only, for the design-fidelity harness (`tools/fidelity`): the scene's clock does
    /// not advance (the backdrop is the still dye either way; the harness's web side shows the
    /// same dye), so two runs shoot the same picture.
    #[arg(long, requires = "smoke")]
    frozen_backdrop: bool,
    /// Hand run only, with `--smoke gallery`: seed the design's Spaces and tabs from this
    /// manifest (`tools/fidelity/parity.mjs` writes it and serves its pages) before the walk.
    #[arg(long, value_name = "JSON", requires = "shot_dir")]
    parity: Option<PathBuf>,
    /// Debugging only: run CEF without its sandbox (logged loudly at start). The engine
    /// reads it from the command line itself (`EngineConfig::from_current_exe`).
    #[arg(long)]
    no_sandbox: bool,
}

/// The `--smoke` value alone; `--url` is joined in by `Command::try_from`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SmokeArg {
    Boot,
    Quit,
    Frames(NonZeroU32),
    Nav,
    Tabs,
    Split,
    Page,
    Parity,
    Block,
    Settings,
    Pages,
    Gallery,
    Session,
    Autofill,
    Windows,
    Media,
    Builtins,
    Video,
    Boost,
    /// `codecs` (the browser's wiring) or `codecs=bare`.
    Codecs {
        wired: bool,
    },
    Ax,
    SessionPhase(Phase),
}

fn parse_smoke(value: &str) -> Result<SmokeArg, String> {
    match value.split_once('=') {
        None if value == "boot" => Ok(SmokeArg::Boot),
        None if value == "quit" => Ok(SmokeArg::Quit),
        None if value == "nav" => Ok(SmokeArg::Nav),
        None if value == "tabs" => Ok(SmokeArg::Tabs),
        None if value == "split" => Ok(SmokeArg::Split),
        None if value == "page" => Ok(SmokeArg::Page),
        None if value == "parity" => Ok(SmokeArg::Parity),
        None if value == "block" => Ok(SmokeArg::Block),
        None if value == "settings" => Ok(SmokeArg::Settings),
        None if value == "pages" => Ok(SmokeArg::Pages),
        None if value == "gallery" => Ok(SmokeArg::Gallery),
        None if value == "session" => Ok(SmokeArg::Session),
        None if value == "autofill" => Ok(SmokeArg::Autofill),
        None if value == "windows" => Ok(SmokeArg::Windows),
        None if value == "media" => Ok(SmokeArg::Media),
        None if value == "builtins" => Ok(SmokeArg::Builtins),
        None if value == "video" => Ok(SmokeArg::Video),
        None if value == "boost" => Ok(SmokeArg::Boost),
        None if value == "codecs" => Ok(SmokeArg::Codecs { wired: true }),
        Some(("codecs", "bare")) => Ok(SmokeArg::Codecs { wired: false }),
        None if value == "ax" => Ok(SmokeArg::Ax),
        Some(("session-phase", "1")) => Ok(SmokeArg::SessionPhase(Phase::Write)),
        Some(("session-phase", "2")) => Ok(SmokeArg::SessionPhase(Phase::Read)),
        Some(("frames", n)) => n
            .parse::<NonZeroU32>()
            .map(SmokeArg::Frames)
            .map_err(|e| format!("frames=N needs a whole number above 0: {e}")),
        _ => Err(
            "expected `boot`, `quit`, `frames=N`, `nav`, `tabs`, `split`, `page`, `parity`, `block`, \
             `settings`, `pages`, `gallery`, `session`, `autofill`, `windows`, `media`, `builtins`, `video`, \
             `boost`, `codecs`, `codecs=bare`, `ax` or `session-phase=1|2`"
                .into(),
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Smoke {
    /// Initialise CEF, print its version, shut down.
    Boot,
    /// Inspect on a selection, DevTools windows shown and counted, one more
    /// asked for just before the quit; the quit is clean (`quit_smoke.rs`).
    Quit,
    /// Show `frames` accelerated frames of `url` (the bundled page if `None`).
    Frames {
        frames: NonZeroU32,
        url: Option<String>,
    },
    /// Load the bundled page, navigate to a second one, wait for its title.
    Nav,
    /// Three tabs on bundled pages: switch, sleep, restore, close.
    Tabs,
    /// A second window: a tab moved into it, both windows' pixels, the window closed.
    Windows,
    /// Two bundled pages side by side: two live views at their panes' sizes, each pane's
    /// pixels, then unsplit.
    Split,
    /// The bundled page's select popup, alert, context menu, find and zoom.
    Page,
    /// The bundled `parity.html`: the checklist of ordinary page interactions (`parity.rs`).
    Parity,
    /// The bundled page requests URLs the smoke's list blocks; they are counted.
    Block,
    /// Open wubble://settings, flip a setting through its toggle, read the store back.
    Settings,
    /// Open history, bookmarks, downloads and passwords on a seeded profile, check their rows.
    Pages,
    /// Walk the shell through its surfaces on a seeded profile, one PNG each into `dir`.
    Gallery { dir: PathBuf },
    /// Runs both session phases on a temporary profile.
    Session,
    /// A loopback sign-in page: fill from the key menu, submit, save, the vault row.
    Autofill,
    /// A loopback page plays a file, an HLS stream and a DRM-marked one: detect, save, join,
    /// check sizes and hashes.
    Media,
    /// Dark mode for pages, reader mode, link hints, screenshots and the PDF viewer on
    /// bundled pages.
    Builtins,
    /// A loopback video page: the video controls, speed per site, SponsorBlock against a fake
    /// endpoint, a media key, the picture-in-picture spike.
    Video,
    /// Boosts on loopback pages: CSS, a tint and a document-start script; a CSP page.
    Boost,
    /// Windowless: which codecs and media sources play, with the browser's blocker, media
    /// observer and page kit (`wired`) or without them (`codecs_smoke.rs`).
    Codecs { wired: bool },
    /// Pages for screen readers: `a11y.web` on, the bundled page's accessible items, a click
    /// and a field's value through them.
    Ax,
    /// One half of `Session`, on the profile in `profile`.
    SessionPhase { phase: Phase, profile: PathBuf },
}

/// The halves of `--smoke session`, each its own process (CEF starts once
/// per process).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Fresh profile: open a second tab, quit (saving the session).
    Write,
    /// Same profile: the two tabs are back, only the visible one live.
    Read,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// The browser, on this page (the bundled one if `None`).
    Run {
        url: Option<String>,
        /// `--profile`, else the per-user profile.
        profile: Option<PathBuf>,
        /// `--downloads`, else the OS downloads folder.
        downloads: Option<PathBuf>,
    },
    Smoke(Smoke),
    /// `--measure`: a hand-run frame-path measurement.
    Measure(Measure),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measure {
    pub scenario: Scenario,
    pub seconds: NonZeroU32,
    /// Seconds to wait before recording, over the scenario's own.
    pub warmup: Option<NonZeroU32>,
    pub out: Option<PathBuf>,
    pub begin_frames: bool,
    pub load: LoadArgs,
}

/// `--measure load`'s own options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadArgs {
    pub variant: Variant,
    pub runs: NonZeroU32,
    /// Empty: the standard pages.
    pub pages: Vec<String>,
    pub serve_only: Option<u16>,
    /// Go back to the last page by history once, after its warm loads.
    pub back: bool,
}

impl Default for LoadArgs {
    fn default() -> Self {
        Self {
            variant: Variant::default(),
            runs: NonZeroU32::new(5).unwrap_or(NonZeroU32::MIN),
            pages: vec![],
            serve_only: None,
            back: false,
        }
    }
}

impl TryFrom<Cli> for Command {
    type Error = clap::Error;

    fn try_from(cli: Cli) -> Result<Self, Self::Error> {
        let conflict = |msg| Err(Cli::command().error(ErrorKind::ArgumentConflict, msg));
        if let Some(scenario) = cli.measure {
            // clap rejects a URL, `--smoke` and `--profile` next to it.
            if scenario != Scenario::Load
                && (cli.load_variant.is_some()
                    || cli.runs.is_some()
                    || !cli.load_page.is_empty()
                    || cli.serve_only.is_some()
                    || cli.load_back)
            {
                return conflict(
                    "--load-variant, --runs, --load-page, --load-back and --serve-only need --measure load",
                );
            }
            let defaults = LoadArgs::default();
            return Ok(Command::Measure(Measure {
                scenario,
                seconds: cli.seconds,
                warmup: cli.warmup,
                out: cli.out,
                begin_frames: cli.begin_frames,
                load: LoadArgs {
                    variant: cli.load_variant.unwrap_or(defaults.variant),
                    runs: cli.runs.unwrap_or(defaults.runs),
                    pages: cli.load_page,
                    serve_only: cli.serve_only,
                    back: cli.load_back,
                },
            }));
        }
        if cli.shot_dir.is_some() != matches!(cli.smoke, Some(SmokeArg::Gallery)) {
            return conflict("--smoke gallery and --shot-dir go together");
        }
        let smoke = match (cli.smoke, cli.url, cli.profile) {
            (None, _, profile) => {
                return Ok(Command::Run {
                    url: cli.page.as_deref().map(wubble_core::url::from_cli_arg),
                    profile,
                    downloads: cli.downloads,
                });
            }
            (Some(SmokeArg::SessionPhase(phase)), None, Some(profile)) => {
                Smoke::SessionPhase { phase, profile }
            }
            (Some(SmokeArg::SessionPhase(_)), _, None) => {
                return Err(Cli::command().error(
                    ErrorKind::MissingRequiredArgument,
                    "--smoke session-phase=N needs --profile",
                ));
            }
            (Some(_), _, Some(_)) => {
                return conflict(
                    "--profile only applies to the browser; smokes use a temporary one",
                );
            }
            (Some(SmokeArg::Frames(frames)), url, None) => Smoke::Frames { frames, url },
            (Some(_), Some(_), None) => return conflict("--url only applies to --smoke frames=N"),
            (Some(SmokeArg::Boot), None, None) => Smoke::Boot,
            (Some(SmokeArg::Quit), None, None) => Smoke::Quit,
            (Some(SmokeArg::Nav), None, None) => Smoke::Nav,
            (Some(SmokeArg::Tabs), None, None) => Smoke::Tabs,
            (Some(SmokeArg::Split), None, None) => Smoke::Split,
            (Some(SmokeArg::Page), None, None) => Smoke::Page,
            (Some(SmokeArg::Parity), None, None) => Smoke::Parity,
            (Some(SmokeArg::Block), None, None) => Smoke::Block,
            (Some(SmokeArg::Settings), None, None) => Smoke::Settings,
            (Some(SmokeArg::Pages), None, None) => Smoke::Pages,
            (Some(SmokeArg::Gallery), None, None) => Smoke::Gallery {
                dir: cli.shot_dir.expect("checked with the smoke above"),
            },
            (Some(SmokeArg::Session), None, None) => Smoke::Session,
            (Some(SmokeArg::Autofill), None, None) => Smoke::Autofill,
            (Some(SmokeArg::Windows), None, None) => Smoke::Windows,
            (Some(SmokeArg::Media), None, None) => Smoke::Media,
            (Some(SmokeArg::Builtins), None, None) => Smoke::Builtins,
            (Some(SmokeArg::Video), None, None) => Smoke::Video,
            (Some(SmokeArg::Boost), None, None) => Smoke::Boost,
            (Some(SmokeArg::Codecs { wired }), None, None) => Smoke::Codecs { wired },
            (Some(SmokeArg::Ax), None, None) => Smoke::Ax,
        };
        Ok(Command::Smoke(smoke))
    }
}

/// Parses `args` (including the program name), or returns clap's error.
#[cfg(test)]
pub fn parse<I, T>(args: I) -> Result<Command, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    parse_with_shot(args).map(|(command, _)| command)
}

/// The hand-run shot options (`--shot`, `--frozen-backdrop`, `--parity`), apart from the
/// command they ride on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShotArgs {
    pub path: Option<PathBuf>,
    pub frozen_backdrop: bool,
    pub parity: Option<PathBuf>,
}

/// [`parse`], and the shot options.
pub fn parse_with_shot<I, T>(args: I) -> Result<(Command, ShotArgs), clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let mut cli = Cli::try_parse_from(args)?;
    let shot = ShotArgs {
        path: cli.shot.take(),
        frozen_backdrop: cli.frozen_backdrop,
        parity: cli.parity.take(),
    };
    Command::try_from(cli).map(|command| (command, shot))
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
            profile: None,
            downloads: None,
        })
    }

    #[test]
    fn no_sandbox_is_accepted_anywhere_and_changes_no_command() {
        assert_eq!(parse(["wubble", "--no-sandbox"]).ok(), run(None));
        assert_eq!(
            parse(["wubble", "--smoke", "boot", "--no-sandbox"]).ok(),
            Some(Command::Smoke(Smoke::Boot))
        );
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
    fn smoke_split() {
        assert_eq!(
            parse(["wubble", "--smoke", "split"]).ok(),
            Some(Command::Smoke(Smoke::Split))
        );
        assert!(parse(["wubble", "--smoke", "split", "--url", "about:blank"]).is_err());
    }

    #[test]
    fn smoke_tabs() {
        assert_eq!(
            parse(["wubble", "--smoke", "tabs"]).ok(),
            Some(Command::Smoke(Smoke::Tabs))
        );
        assert!(parse(["wubble", "--smoke", "tabs", "--url", "about:blank"]).is_err());
        assert!(parse(["wubble", "--smoke", "tabs=2"]).is_err());
    }

    #[test]
    fn smoke_page() {
        assert_eq!(
            parse(["wubble", "--smoke", "page"]).ok(),
            Some(Command::Smoke(Smoke::Page))
        );
        assert!(parse(["wubble", "--smoke", "page", "--url", "about:blank"]).is_err());
        assert_eq!(
            parse(["wubble", "--smoke", "parity"]).ok(),
            Some(Command::Smoke(Smoke::Parity))
        );
    }

    #[test]
    fn smoke_block() {
        assert_eq!(
            parse(["wubble", "--smoke", "block"]).ok(),
            Some(Command::Smoke(Smoke::Block))
        );
        assert!(parse(["wubble", "--smoke", "block", "--url", "about:blank"]).is_err());
    }

    #[test]
    fn smoke_settings() {
        assert_eq!(
            parse(["wubble", "--smoke", "settings"]).ok(),
            Some(Command::Smoke(Smoke::Settings))
        );
        assert!(parse(["wubble", "--smoke", "settings", "--url", "about:blank"]).is_err());
        assert!(parse(["wubble", "--smoke", "settings", "--profile", "/tmp/p"]).is_err());
    }

    #[test]
    fn a_shot_needs_a_smoke() {
        let (command, shot) =
            parse_with_shot(["wubble", "--smoke", "settings", "--shot", "/tmp/s.png"]).unwrap();
        assert_eq!(command, Command::Smoke(Smoke::Settings));
        assert_eq!(shot.path, Some(PathBuf::from("/tmp/s.png")));
        assert!(!shot.frozen_backdrop);
        assert!(parse(["wubble", "--shot", "/tmp/s.png"]).is_err());
    }

    #[test]
    fn the_fidelity_flags_ride_on_a_smoke() {
        let args = [
            "wubble",
            "--smoke",
            "gallery",
            "--shot-dir",
            "/tmp/g",
            "--frozen-backdrop",
            "--parity",
            "/tmp/p.json",
        ];
        let (_, shot) = parse_with_shot(args).unwrap();
        assert!(shot.frozen_backdrop);
        assert_eq!(shot.parity, Some(PathBuf::from("/tmp/p.json")));
        assert!(parse(["wubble", "--frozen-backdrop"]).is_err());
        assert!(parse(["wubble", "--smoke", "boot", "--parity", "/tmp/p.json"]).is_err());
    }

    #[test]
    fn smoke_pages() {
        assert_eq!(
            parse(["wubble", "--smoke", "pages"]).ok(),
            Some(Command::Smoke(Smoke::Pages))
        );
        assert!(parse(["wubble", "--smoke", "pages", "--url", "about:blank"]).is_err());
        assert!(parse(["wubble", "--smoke", "pages", "--profile", "/tmp/p"]).is_err());
    }

    #[test]
    fn the_gallery_needs_its_folder_and_nothing_else_takes_it() {
        assert_eq!(
            parse(["wubble", "--smoke", "gallery", "--shot-dir", "/tmp/g"]).ok(),
            Some(Command::Smoke(Smoke::Gallery {
                dir: PathBuf::from("/tmp/g")
            }))
        );
        assert!(parse(["wubble", "--smoke", "gallery"]).is_err());
        assert!(parse(["wubble", "--smoke", "pages", "--shot-dir", "/tmp/g"]).is_err());
        assert!(parse(["wubble", "--shot-dir", "/tmp/g"]).is_err());
        let both = [
            "wubble",
            "--smoke",
            "gallery",
            "--shot-dir",
            "/tmp/g",
            "--shot",
            "/tmp/s.png",
        ];
        assert!(parse(both).is_err());
    }

    #[test]
    fn smoke_builtins() {
        assert_eq!(
            parse(["wubble", "--smoke", "builtins"]).ok(),
            Some(Command::Smoke(Smoke::Builtins))
        );
        assert!(parse(["wubble", "--smoke", "builtins", "--url", "about:blank"]).is_err());
    }

    #[test]
    fn smoke_video_and_boost() {
        assert_eq!(
            parse(["wubble", "--smoke", "video"]).ok(),
            Some(Command::Smoke(Smoke::Video))
        );
        assert_eq!(
            parse(["wubble", "--smoke", "boost"]).ok(),
            Some(Command::Smoke(Smoke::Boost))
        );
        assert!(parse(["wubble", "--smoke", "video", "--url", "about:blank"]).is_err());
        assert_eq!(
            parse(["wubble", "--smoke", "ax"]).ok(),
            Some(Command::Smoke(Smoke::Ax))
        );
    }

    #[test]
    fn smoke_codecs() {
        assert_eq!(
            parse(["wubble", "--smoke", "codecs"]).ok(),
            Some(Command::Smoke(Smoke::Codecs { wired: true }))
        );
        assert_eq!(
            parse(["wubble", "--smoke", "codecs=bare", "--no-sandbox"]).ok(),
            Some(Command::Smoke(Smoke::Codecs { wired: false }))
        );
        assert!(parse(["wubble", "--smoke", "codecs=other"]).is_err());
    }

    #[test]
    fn a_url_does_not_combine_with_a_smoke() {
        for smoke in [
            "boot", "nav", "tabs", "split", "page", "block", "settings", "pages", "windows",
            "frames=3", "session",
        ] {
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
        assert_eq!(
            parse(["wubble", "--smoke", "quit"]).ok(),
            Some(Command::Smoke(Smoke::Quit))
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
    fn profile_overrides_the_per_user_one() {
        assert_eq!(
            parse(["wubble", "--profile", "/tmp/p", "example.com"]).ok(),
            Some(Command::Run {
                url: Some("https://example.com/".into()),
                profile: Some("/tmp/p".into()),
                downloads: None,
            })
        );
        for smoke in [
            "boot", "nav", "tabs", "split", "page", "frames=3", "session",
        ] {
            assert!(
                parse(["wubble", "--smoke", smoke, "--profile", "/tmp/p"]).is_err(),
                "{smoke}"
            );
        }
    }

    #[test]
    fn smoke_session_and_its_phases() {
        assert_eq!(
            parse(["wubble", "--smoke", "session"]).ok(),
            Some(Command::Smoke(Smoke::Session))
        );
        for (arg, phase) in [
            ("session-phase=1", Phase::Write),
            ("session-phase=2", Phase::Read),
        ] {
            assert_eq!(
                parse(["wubble", "--smoke", arg, "--profile", "/tmp/p"]).ok(),
                Some(Command::Smoke(Smoke::SessionPhase {
                    phase,
                    profile: "/tmp/p".into()
                })),
                "{arg}"
            );
            assert!(
                parse(["wubble", "--smoke", arg]).is_err(),
                "{arg} needs a profile"
            );
        }
        for bad in ["session-phase=3", "session-phase=", "session=1"] {
            assert!(
                parse(["wubble", "--smoke", bad, "--profile", "/tmp/p"]).is_err(),
                "{bad}"
            );
        }
        assert!(parse(["wubble", "--smoke", "session", "--url", "about:blank"]).is_err());
    }

    fn measure(scenario: Scenario, seconds: u32, out: Option<&str>) -> Option<Command> {
        Some(Command::Measure(Measure {
            scenario,
            seconds: NonZeroU32::new(seconds).expect("test uses seconds > 0"),
            warmup: None,
            out: out.map(PathBuf::from),
            begin_frames: false,
            load: LoadArgs::default(),
        }))
    }

    #[test]
    fn measure_scenarios() {
        for (arg, scenario) in [
            ("scroll", Scenario::Scroll),
            ("video", Scenario::Video),
            ("resize", Scenario::Resize),
            ("typing", Scenario::Typing),
            ("blocker", Scenario::Blocker),
            ("idle", Scenario::Idle),
            ("startup", Scenario::Startup),
            ("load", Scenario::Load),
            ("memory", Scenario::Memory),
        ] {
            assert_eq!(
                parse(["wubble", "--measure", arg]).ok(),
                measure(scenario, 10, None),
                "{arg}"
            );
        }
        assert_eq!(
            parse(["wubble", "--measure", "idle", "--warmup", "20"])
                .ok()
                .and_then(|c| match c {
                    Command::Measure(m) => m.warmup,
                    _ => None,
                }),
            NonZeroU32::new(20)
        );
        assert!(parse(["wubble", "--warmup", "20"]).is_err());
        assert!(parse(["wubble", "--measure", "idle", "--warmup", "0"]).is_err());
        assert_eq!(
            parse([
                "wubble",
                "--measure",
                "typing",
                "--seconds",
                "3",
                "--out",
                "/tmp/m.csv"
            ])
            .ok(),
            measure(Scenario::Typing, 3, Some("/tmp/m.csv"))
        );
        assert_eq!(
            parse(["wubble", "--measure", "video", "--begin-frames"]).ok(),
            Some(Command::Measure(Measure {
                scenario: Scenario::Video,
                seconds: DEFAULT_SECONDS,
                warmup: None,
                out: None,
                begin_frames: true,
                load: LoadArgs::default(),
            }))
        );
    }

    #[test]
    fn load_options_need_the_load_scenario() {
        let Some(Command::Measure(m)) = parse([
            "wubble",
            "--measure",
            "load",
            "--load-variant",
            "no-hook",
            "--runs",
            "3",
            "--load-page",
            "/spec",
            "--serve-only",
            "0",
            "--load-back",
        ])
        .ok() else {
            panic!("--measure load with its options parses");
        };
        assert_eq!(m.load.variant, Variant::NoHook);
        assert_eq!(m.load.runs.get(), 3);
        assert_eq!(m.load.pages, ["/spec"]);
        assert_eq!(m.load.serve_only, Some(0));
        assert!(m.load.back);
        for bad in [
            &["--measure", "scroll", "--load-variant", "no-hook"][..],
            &["--measure", "scroll", "--runs", "3"],
            &["--measure", "idle", "--load-page", "/spec"],
            &["--measure", "idle", "--serve-only", "0"],
            &["--measure", "idle", "--load-back"],
            &["--measure", "load", "--runs", "0"],
            &["--measure", "load", "--load-variant", "nope"],
            &["--load-variant", "no-hook"],
            &["--runs", "3"],
        ] {
            let args = std::iter::once("wubble").chain(bad.iter().copied());
            assert!(parse(args).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn bad_measure_arguments_are_errors() {
        for bad in [
            &["--measure", "nope"][..],
            &["--measure"],
            &["--measure", "scroll", "--seconds", "0"],
            &["--measure", "scroll", "--seconds", "x"],
            &["--measure", "scroll", "example.com"],
            &["--measure", "scroll", "--smoke", "boot"],
            &["--measure", "scroll", "--profile", "/tmp/p"],
            &["--measure", "scroll", "--url", "about:blank"],
            &["--seconds", "3"],
            &["--out", "/tmp/m.csv"],
            &["--begin-frames"],
        ] {
            let args = std::iter::once("wubble").chain(bad.iter().copied());
            assert!(parse(args).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn downloads_folder_for_the_browser_only() {
        assert_eq!(
            parse(["wubble", "--downloads", "/tmp/d"]).ok(),
            Some(Command::Run {
                url: None,
                profile: None,
                downloads: Some("/tmp/d".into()),
            })
        );
        assert!(parse(["wubble", "--smoke", "page", "--downloads", "/tmp/d"]).is_err());
    }

    #[test]
    fn url_needs_a_frames_smoke() {
        assert!(parse(["wubble", "--url", "about:blank"]).is_err());
        assert!(parse(["wubble", "--smoke", "boot", "--url", "about:blank"]).is_err());
    }
}
