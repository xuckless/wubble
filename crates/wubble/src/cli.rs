//! Command-line arguments, parsed declaratively into one [`Command`].

use std::num::NonZeroU32;
use std::path::PathBuf;

use crate::passwords_smoke::VaultPhase;
use clap::{CommandFactory, Parser, error::ErrorKind};
use wubble_core::page_layer;
pub use wubble_core::page_layer::Mode as PageLayerMode;
pub use wubble_gfx::pacing::Pacing;

pub use crate::load_run::Variant;
pub use crate::measure::Scenario;

/// `wubble --help` and `wubble mcp --help`: how an agent uses wubble.
const AGENT_USE: &str = r#"Agent use: wubble hosts an MCP server for AI agents on this computer once you turn it on in
Settings > Agents (MCP) (off by default). An agent harness starts `wubble mcp`, which speaks MCP
on stdio and relays to the running browser's private socket (no TCP port); it finds the
browser by itself. Generic server entry: {"command": "<path to wubble>", "args": ["mcp"]}.
Examples: `claude mcp add wubble -- <path to wubble> mcp`; `codex mcp add wubble -- <path to
wubble> mcp`; `gemini mcp add -s user wubble <path to wubble> mcp`. Turning hosting on also
adds that entry to the harnesses it finds (Settings lists them). Every session needs your
approval on screen, so the first call, start_session, can block for minutes: give the server a
long tool timeout."#;

/// `--measure`'s run time unless `--seconds` says otherwise.
const DEFAULT_SECONDS: NonZeroU32 = NonZeroU32::new(10).unwrap();

#[derive(Parser, Debug)]
#[command(
    name = "wubble",
    about = "The wubble browser.",
    version,
    args_conflicts_with_subcommands = true,
    after_help = AGENT_USE
)]
struct Cli {
    #[command(subcommand)]
    sub: Option<Sub>,
    /// Page to open (`example.com` means `https://example.com`); the bundled
    /// page if none.
    #[arg(value_name = "URL", conflicts_with_all = ["smoke", "measure"])]
    page: Option<String>,
    /// Run a self-exiting check instead of the browser: `boot` (CEF starts and
    /// stops, no window), `update` (the updater against a signed fake release on loopback, no
    /// window, no CEF), `frames=N` (show N zero-copy frames, then exit; on
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
    /// discarded) or `tabs` (three tabs, one animating, switched every second: the time to the
    /// new tab's first frame and the frames of the switch). Exits by itself.
    #[arg(long, value_name = "SCENARIO", value_enum, conflicts_with = "profile")]
    measure: Option<Scenario>,
    /// How long `--measure` runs.
    #[arg(long, value_name = "N", requires = "measure", default_value_t = DEFAULT_SECONDS)]
    seconds: NonZeroU32,
    /// How long `--measure` waits after the first frame before it records, instead of the
    /// scenario's own (`idle`: long enough for the scene to have settled).
    #[arg(long, value_name = "N", requires = "measure")]
    warmup: Option<NonZeroU32>,
    /// `--measure idle` on this page instead of the bundled still one (the start page,
    /// `wubble://start`; a page with a caret or a small animation), to see what it keeps drawing.
    #[arg(long, value_name = "URL", requires = "measure")]
    measure_page: Option<String>,
    /// CEF paced by the shell's external begin frames, one per render of the window, instead of
    /// this platform's default (`wubble_gfx::pacing::PLATFORM_PACING`); for any windowed run,
    /// `--measure` included.
    #[arg(long, conflicts_with = "no_begin_frames")]
    begin_frames: bool,
    /// CEF painting on its own timer, instead of this platform's default.
    #[arg(long)]
    no_begin_frames: bool,
    /// Which pages show on their own OS layer instead of through the shell
    /// (`wubble_core::page_layer`): `off` (none, the default) or `fullscreen` (a page in
    /// fullscreen and the picture-in-picture window; macOS only, elsewhere the same as off).
    #[arg(long, value_name = "MODE", value_parser = parse_page_layer, default_value = "off")]
    page_layer: PageLayerMode,
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
    /// Page for `--smoke frames=N` instead of the bundled test page; the page
    /// `--smoke autofill=probe` looks at.
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

#[derive(clap::Subcommand, Debug)]
enum Sub {
    /// Speak MCP on stdio for an agent harness, relayed to the running browser's private socket
    /// (Settings > Agents (MCP), off by default). Finds the hosting browser by itself; starts no
    /// browser.
    #[command(after_help = AGENT_USE)]
    Mcp {
        /// The browser's profile directory, to choose one of several hosting browsers.
        #[arg(long, value_name = "DIR")]
        profile: Option<PathBuf>,
        /// Print the hosting browsers (profile, socket, version, pid) and exit.
        #[arg(long, conflicts_with = "profile")]
        list: bool,
    },
}

/// The `--smoke` value alone; `--url` is joined in by `Command::try_from`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SmokeArg {
    Boot,
    Update,
    Quit,
    Spell,
    Frames(NonZeroU32),
    Nav,
    Tabs,
    Split,
    Pinch,
    Page,
    Parity,
    Dnd,
    Block,
    Settings,
    Pages,
    Gallery,
    Session,
    Autofill,
    AutofillProbe,
    Windows,
    Popup,
    Media,
    MediaProbe,
    Builtins,
    Video,
    Boost,
    /// `codecs` (the browser's wiring) or `codecs=bare`.
    Codecs {
        wired: bool,
    },
    Ax,
    Mcp,
    /// `overlay` or `overlay=dark`.
    Overlay {
        dark: bool,
    },
    Start,
    SessionPhase(Phase),
    Passwords,
    PasswordsPhase(VaultPhase),
    Data,
    DataPhase(crate::data_smoke::Ending),
    /// `extensions` or `extensions-none`.
    Extensions {
        installed: bool,
    },
}

/// `--page-layer`: one of `page_layer::MODES`.
fn parse_page_layer(value: &str) -> Result<PageLayerMode, String> {
    page_layer::mode(value).ok_or_else(|| {
        let names: Vec<&str> = page_layer::MODES.iter().map(|&(n, _)| n).collect();
        format!("one of {}", names.join(", "))
    })
}

fn parse_smoke(value: &str) -> Result<SmokeArg, String> {
    match value.split_once('=') {
        None if value == "boot" => Ok(SmokeArg::Boot),
        None if value == "update" => Ok(SmokeArg::Update),
        None if value == "quit" => Ok(SmokeArg::Quit),
        None if value == "spell" => Ok(SmokeArg::Spell),
        None if value == "nav" => Ok(SmokeArg::Nav),
        None if value == "tabs" => Ok(SmokeArg::Tabs),
        None if value == "split" => Ok(SmokeArg::Split),
        None if value == "pinch" => Ok(SmokeArg::Pinch),
        None if value == "dnd" => Ok(SmokeArg::Dnd),
        None if value == "page" => Ok(SmokeArg::Page),
        None if value == "parity" => Ok(SmokeArg::Parity),
        None if value == "block" => Ok(SmokeArg::Block),
        None if value == "settings" => Ok(SmokeArg::Settings),
        None if value == "pages" => Ok(SmokeArg::Pages),
        None if value == "gallery" => Ok(SmokeArg::Gallery),
        None if value == "session" => Ok(SmokeArg::Session),
        None if value == "autofill" => Ok(SmokeArg::Autofill),
        Some(("autofill", "probe")) => Ok(SmokeArg::AutofillProbe),
        None if value == "windows" => Ok(SmokeArg::Windows),
        None if value == "popup" => Ok(SmokeArg::Popup),
        None if value == "media" => Ok(SmokeArg::Media),
        Some(("media", "probe")) => Ok(SmokeArg::MediaProbe),
        None if value == "builtins" => Ok(SmokeArg::Builtins),
        None if value == "video" => Ok(SmokeArg::Video),
        None if value == "boost" => Ok(SmokeArg::Boost),
        None if value == "codecs" => Ok(SmokeArg::Codecs { wired: true }),
        Some(("codecs", "bare")) => Ok(SmokeArg::Codecs { wired: false }),
        None if value == "ax" => Ok(SmokeArg::Ax),
        None if value == "extensions" => Ok(SmokeArg::Extensions { installed: true }),
        None if value == "extensions-none" => Ok(SmokeArg::Extensions { installed: false }),
        None if value == "mcp" => Ok(SmokeArg::Mcp),
        None if value == "start" => Ok(SmokeArg::Start),
        None if value == "overlay" => Ok(SmokeArg::Overlay { dark: false }),
        Some(("overlay", "dark")) => Ok(SmokeArg::Overlay { dark: true }),
        Some(("session-phase", "1")) => Ok(SmokeArg::SessionPhase(Phase::Write)),
        Some(("session-phase", "2")) => Ok(SmokeArg::SessionPhase(Phase::Read)),
        None if value == "passwords" => Ok(SmokeArg::Passwords),
        Some(("passwords-phase", "1")) => Ok(SmokeArg::PasswordsPhase(VaultPhase::Write)),
        Some(("passwords-phase", "2")) => Ok(SmokeArg::PasswordsPhase(VaultPhase::Read)),
        Some(("passwords-phase", "3")) => Ok(SmokeArg::PasswordsPhase(VaultPhase::Broken)),
        Some(("passwords-phase", "4")) => Ok(SmokeArg::PasswordsPhase(VaultPhase::Lost)),
        None if value == "data" => Ok(SmokeArg::Data),
        Some(("data-phase", "quit")) => Ok(SmokeArg::DataPhase(crate::data_smoke::Ending::Quit)),
        Some(("data-phase", "kill")) => Ok(SmokeArg::DataPhase(crate::data_smoke::Ending::Kill)),
        Some(("frames", n)) => n
            .parse::<NonZeroU32>()
            .map(SmokeArg::Frames)
            .map_err(|e| format!("frames=N needs a whole number above 0: {e}")),
        _ => Err(
            "expected `boot`, `update`, `quit`, `spell`, `frames=N`, `nav`, `tabs`, `split`, `pinch`, `page`, `parity`, `dnd`, `block`, \
             `settings`, `pages`, `gallery`, `session`, `autofill`, `autofill=probe`, `windows`, `media`, `media=probe`, `builtins`, `video`, \
             `boost`, `codecs`, `codecs=bare`, `ax`, `mcp`, `overlay`, `overlay=dark`, `start`, `extensions`, \
             `extensions-none`, `data` or `session-phase=1|2`"
                .into(),
        ),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Smoke {
    /// Initialise CEF, print its version, shut down.
    Boot,
    /// Windowless, no CEF: the updater against a signed fake release on loopback, staged into
    /// a temporary install root; a tampered archive refused (`update_smoke.rs`).
    Update,
    /// Inspect on a selection, DevTools windows shown and counted, one more
    /// asked for just before the quit; the quit is clean (`quit_smoke.rs`).
    Quit,
    /// A misspelt word in a textarea on loopback is marked, with suggestions, from the bundled
    /// dictionary and no request off the machine (`spell_smoke.rs`).
    Spell,
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
    /// Pages opening pop-up windows and tabs through `window.open` (`popup_smoke.rs`).
    Popup,
    /// Two bundled pages side by side: two live views at their panes' sizes, each pane's
    /// pixels, then unsplit.
    Split,
    /// A trackpad pinch through the host's pinch path on a loopback page: Blink's visual
    /// viewport scale, never the page zoom; a page keeping the Ctrl+wheel; split panes apart.
    Pinch,
    /// The bundled page's select popup, alert, context menu, find and zoom.
    Page,
    /// The bundled `parity.html`: the checklist of ordinary page interactions (`parity.rs`).
    Parity,
    /// The bundled `dnd.html`: a page's drags inside the window (`dnd_smoke.rs`).
    /// `--shot-dir DIR`: a PNG of the window mid-drag, per item that asks.
    Dnd { dir: Option<PathBuf> },
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
    /// The sign-in corpus on loopback: fill from the key menu, submit, save, the vault rows.
    /// With `probe`, a real page instead, read-only: what the chip would offer (by hand).
    Autofill { probe: Option<String> },
    /// A loopback page plays a file, an HLS stream and a DRM-marked one: detect, save, join,
    /// check sizes and hashes. With `probe`, a page instead, by hand: what the downloader lists,
    /// and (`WUBBLE_PROBE_SAVE`) what each save does.
    Media { probe: Option<String> },
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
    /// Agent sessions over MCP: a client on the browser's socket starts a headless session
    /// and drives the bundled loopback page through the tools (`mcp_smoke.rs`).
    Mcp,
    /// Overlay glass over the bundled green and red page (`dark`: the dark one) frosts the
    /// page; the window's glass picks up its light (`overlay_smoke.rs`). `dir`: `--shot-dir`,
    /// one PNG per step.
    Overlay { dark: bool, dir: Option<PathBuf> },
    /// One half of `Session`, on the profile in `profile`.
    SessionPhase { phase: Phase, profile: PathBuf },
    /// The passwords page across restarts on a temporary profile (`passwords_smoke.rs`).
    Passwords,
    /// One process of `Passwords`, on the profile in `profile`.
    PasswordsPhase { phase: VaultPhase, profile: PathBuf },
    /// The built-in pages and their data on a loopback server, one run that quits and one that
    /// is killed, the database compared with the pages after each (`data_smoke.rs`).
    Data,
    /// One run of `Data`, on the profile in `profile`.
    DataPhase {
        ending: crate::data_smoke::Ending,
        profile: PathBuf,
    },
    /// The start page: its bubble drawn, poked, a mini popped, the capsule's command bar, a
    /// tile; one PNG per step into `dir` when given (`start_smoke.rs`).
    Start { dir: Option<PathBuf> },
    /// Windowless: the test extension installed as a directory and as a .crx, its content
    /// script and service worker on a loopback page (`installed`), or nothing installed and no
    /// extension process (`extensions_smoke.rs`).
    Extensions { installed: bool },
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
    /// `wubble mcp`: the stdio relay to the browser's MCP socket on this profile.
    Mcp {
        profile: Option<PathBuf>,
        /// `--list`: print the hosting browsers instead.
        list: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measure {
    pub scenario: Scenario,
    pub seconds: NonZeroU32,
    /// Seconds to wait before recording, over the scenario's own.
    pub warmup: Option<NonZeroU32>,
    pub out: Option<PathBuf>,
    pub load: LoadArgs,
    /// `--measure idle`'s page instead of the bundled one.
    pub page: Option<String>,
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
        if let Some(Sub::Mcp { profile, list }) = cli.sub {
            return Ok(Command::Mcp { profile, list });
        }
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
            if scenario != Scenario::Idle && cli.measure_page.is_some() {
                return conflict("--measure-page needs --measure idle");
            }
            let defaults = LoadArgs::default();
            return Ok(Command::Measure(Measure {
                scenario,
                seconds: cli.seconds,
                warmup: cli.warmup,
                out: cli.out,
                page: cli
                    .measure_page
                    .as_deref()
                    .map(wubble_core::url::from_cli_arg),
                load: LoadArgs {
                    variant: cli.load_variant.unwrap_or(defaults.variant),
                    runs: cli.runs.unwrap_or(defaults.runs),
                    pages: cli.load_page,
                    serve_only: cli.serve_only,
                    back: cli.load_back,
                },
            }));
        }
        let gallery = matches!(cli.smoke, Some(SmokeArg::Gallery));
        let overlay = matches!(
            cli.smoke,
            Some(SmokeArg::Overlay { .. } | SmokeArg::Start | SmokeArg::Dnd)
        );
        if cli.shot_dir.is_some() != gallery && !overlay {
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
            (Some(SmokeArg::DataPhase(ending)), None, Some(profile)) => {
                Smoke::DataPhase { ending, profile }
            }
            (Some(SmokeArg::DataPhase(_)), _, None) => {
                return Err(Cli::command().error(
                    ErrorKind::MissingRequiredArgument,
                    "--smoke data-phase=quit|kill needs --profile",
                ));
            }
            (Some(SmokeArg::SessionPhase(_)), _, None) => {
                return Err(Cli::command().error(
                    ErrorKind::MissingRequiredArgument,
                    "--smoke session-phase=N needs --profile",
                ));
            }
            (Some(SmokeArg::PasswordsPhase(phase)), None, Some(profile)) => {
                Smoke::PasswordsPhase { phase, profile }
            }
            (Some(SmokeArg::PasswordsPhase(_)), _, None) => {
                return Err(Cli::command().error(
                    ErrorKind::MissingRequiredArgument,
                    "--smoke passwords-phase=N needs --profile",
                ));
            }
            (Some(_), _, Some(_)) => {
                return conflict(
                    "--profile only applies to the browser; smokes use a temporary one",
                );
            }
            (Some(SmokeArg::Frames(frames)), url, None) => Smoke::Frames { frames, url },
            (Some(SmokeArg::AutofillProbe), Some(url), None) => Smoke::Autofill {
                probe: Some(wubble_core::url::from_cli_arg(&url)),
            },
            (Some(SmokeArg::AutofillProbe), None, None) => {
                return Err(Cli::command().error(
                    ErrorKind::MissingRequiredArgument,
                    "--smoke autofill=probe needs --url",
                ));
            }
            (Some(SmokeArg::MediaProbe), Some(url), None) => Smoke::Media {
                probe: Some(wubble_core::url::from_cli_arg(&url)),
            },
            (Some(SmokeArg::MediaProbe), None, None) => {
                return Err(Cli::command().error(
                    ErrorKind::MissingRequiredArgument,
                    "--smoke media=probe needs --url",
                ));
            }
            (Some(_), Some(_), None) => {
                return conflict("--url only applies to --smoke frames=N and the probes");
            }
            (Some(SmokeArg::Boot), None, None) => Smoke::Boot,
            (Some(SmokeArg::Update), None, None) => Smoke::Update,
            (Some(SmokeArg::Quit), None, None) => Smoke::Quit,
            (Some(SmokeArg::Spell), None, None) => Smoke::Spell,
            (Some(SmokeArg::Nav), None, None) => Smoke::Nav,
            (Some(SmokeArg::Tabs), None, None) => Smoke::Tabs,
            (Some(SmokeArg::Split), None, None) => Smoke::Split,
            (Some(SmokeArg::Pinch), None, None) => Smoke::Pinch,
            (Some(SmokeArg::Page), None, None) => Smoke::Page,
            (Some(SmokeArg::Parity), None, None) => Smoke::Parity,
            (Some(SmokeArg::Dnd), None, None) => Smoke::Dnd { dir: cli.shot_dir },
            (Some(SmokeArg::Block), None, None) => Smoke::Block,
            (Some(SmokeArg::Settings), None, None) => Smoke::Settings,
            (Some(SmokeArg::Pages), None, None) => Smoke::Pages,
            (Some(SmokeArg::Gallery), None, None) => Smoke::Gallery {
                dir: cli.shot_dir.expect("checked with the smoke above"),
            },
            (Some(SmokeArg::Session), None, None) => Smoke::Session,
            (Some(SmokeArg::Passwords), None, None) => Smoke::Passwords,
            (Some(SmokeArg::Autofill), None, None) => Smoke::Autofill { probe: None },
            (Some(SmokeArg::Data), None, None) => Smoke::Data,
            (Some(SmokeArg::Windows), None, None) => Smoke::Windows,
            (Some(SmokeArg::Popup), None, None) => Smoke::Popup,
            (Some(SmokeArg::Media), None, None) => Smoke::Media { probe: None },
            (Some(SmokeArg::Builtins), None, None) => Smoke::Builtins,
            (Some(SmokeArg::Video), None, None) => Smoke::Video,
            (Some(SmokeArg::Boost), None, None) => Smoke::Boost,
            (Some(SmokeArg::Codecs { wired }), None, None) => Smoke::Codecs { wired },
            (Some(SmokeArg::Ax), None, None) => Smoke::Ax,
            (Some(SmokeArg::Mcp), None, None) => Smoke::Mcp,
            (Some(SmokeArg::Overlay { dark }), None, None) => Smoke::Overlay {
                dark,
                dir: cli.shot_dir,
            },
            (Some(SmokeArg::Start), None, None) => Smoke::Start { dir: cli.shot_dir },
            (Some(SmokeArg::Extensions { installed }), None, None) => {
                Smoke::Extensions { installed }
            }
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
    parse_with_shot(args).map(|(command, ..)| command)
}

/// The hand-run shot options (`--shot`, `--frozen-backdrop`, `--parity`), apart from the
/// command they ride on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShotArgs {
    pub path: Option<PathBuf>,
    pub frozen_backdrop: bool,
    pub parity: Option<PathBuf>,
}

/// What any run takes from the command line, beside its command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunFlags {
    /// The pacing `--begin-frames` or `--no-begin-frames` asks for, if either.
    pub pacing: Option<Pacing>,
    /// `--page-layer`.
    pub page_layer: PageLayerMode,
}

/// [`parse`], the shot options and the run's flags.
pub fn parse_with_shot<I, T>(args: I) -> Result<(Command, ShotArgs, RunFlags), clap::Error>
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
    let pacing = match (cli.begin_frames, cli.no_begin_frames) {
        (true, _) => Some(Pacing::Shell),
        (_, true) => Some(Pacing::CefTimer),
        _ => None,
    };
    let flags = RunFlags {
        pacing,
        page_layer: cli.page_layer,
    };
    Command::try_from(cli).map(|command| (command, shot, flags))
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
    fn smoke_start() {
        assert_eq!(
            parse(["wubble", "--smoke", "start"]).ok(),
            Some(Command::Smoke(Smoke::Start { dir: None }))
        );
        assert_eq!(
            parse(["wubble", "--smoke", "start", "--shot-dir", "/tmp/d"]).ok(),
            Some(Command::Smoke(Smoke::Start {
                dir: Some(PathBuf::from("/tmp/d"))
            }))
        );
        assert!(parse(["wubble", "--smoke", "start", "--profile", "/tmp/p"]).is_err());
    }

    #[test]
    fn a_shot_needs_a_smoke() {
        let (command, shot, _) =
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
        let (_, shot, _) = parse_with_shot(args).unwrap();
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
    fn mcp_is_the_relay_and_takes_a_profile() {
        assert_eq!(
            parse(["wubble", "mcp"]).ok(),
            Some(Command::Mcp {
                profile: None,
                list: false
            })
        );
        assert_eq!(
            parse(["wubble", "mcp", "--profile", "/tmp/p"]).ok(),
            Some(Command::Mcp {
                list: false,
                profile: Some(PathBuf::from("/tmp/p"))
            })
        );
        assert!(parse(["wubble", "mcp", "--smoke", "boot"]).is_err());
        assert_eq!(
            parse(["wubble", "example.com/p"]).ok(),
            run(Some("https://example.com/p"))
        );
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
        assert_eq!(
            parse(["wubble", "--smoke", "mcp"]).ok(),
            Some(Command::Smoke(Smoke::Mcp))
        );
        assert_eq!(
            parse(["wubble", "--smoke", "overlay"]).ok(),
            Some(Command::Smoke(Smoke::Overlay {
                dark: false,
                dir: None
            }))
        );
        assert_eq!(
            parse(["wubble", "--smoke", "overlay=dark", "--shot-dir", "/tmp/o"]).ok(),
            Some(Command::Smoke(Smoke::Overlay {
                dark: true,
                dir: Some(PathBuf::from("/tmp/o"))
            }))
        );
    }

    #[test]
    fn smoke_extensions() {
        assert_eq!(
            parse(["wubble", "--smoke", "extensions"]).ok(),
            Some(Command::Smoke(Smoke::Extensions { installed: true }))
        );
        assert_eq!(
            parse(["wubble", "--smoke", "extensions-none"]).ok(),
            Some(Command::Smoke(Smoke::Extensions { installed: false }))
        );
        assert!(parse(["wubble", "--smoke", "extensions", "--url", "about:blank"]).is_err());
        assert!(parse(["wubble", "--smoke", "extensions=x"]).is_err());
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
            "frames=3", "session", "start",
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
        assert_eq!(
            parse(["wubble", "--smoke", "spell"]).ok(),
            Some(Command::Smoke(Smoke::Spell))
        );
        assert_eq!(
            parse(["wubble", "--smoke", "update"]).ok(),
            Some(Command::Smoke(Smoke::Update))
        );
    }

    /// `--version` prints the workspace version (clap's "display version" exit).
    #[test]
    fn version_is_the_workspaces() {
        let e = parse(["wubble", "--version"]).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::DisplayVersion);
        assert_eq!(
            e.to_string().trim(),
            format!("wubble {}", env!("CARGO_PKG_VERSION"))
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

    #[test]
    fn smoke_data_and_its_runs() {
        use crate::data_smoke::Ending;
        assert_eq!(
            parse(["wubble", "--smoke", "data"]).ok(),
            Some(Command::Smoke(Smoke::Data))
        );
        for (arg, ending) in [
            ("data-phase=quit", Ending::Quit),
            ("data-phase=kill", Ending::Kill),
        ] {
            assert_eq!(
                parse(["wubble", "--smoke", arg, "--profile", "/tmp/p"]).ok(),
                Some(Command::Smoke(Smoke::DataPhase {
                    ending,
                    profile: "/tmp/p".into()
                })),
                "{arg}"
            );
            assert!(
                parse(["wubble", "--smoke", arg]).is_err(),
                "{arg} needs a profile"
            );
        }
        assert!(parse(["wubble", "--smoke", "data", "--profile", "/tmp/p"]).is_err());
        assert!(
            parse([
                "wubble",
                "--smoke",
                "data-phase=stop",
                "--profile",
                "/tmp/p"
            ])
            .is_err()
        );
    }

    fn measure(scenario: Scenario, seconds: u32, out: Option<&str>) -> Option<Command> {
        Some(Command::Measure(Measure {
            scenario,
            seconds: NonZeroU32::new(seconds).expect("test uses seconds > 0"),
            warmup: None,
            out: out.map(PathBuf::from),
            load: LoadArgs::default(),
            page: None,
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
            ("tabs", Scenario::Tabs),
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
            measure(Scenario::Video, DEFAULT_SECONDS.get(), None)
        );
    }

    #[test]
    fn pacing_is_overridden_both_ways_for_any_run() {
        let pacing = |args: &[&str]| {
            parse_with_shot(std::iter::once("wubble").chain(args.iter().copied()))
                .ok()
                .map(|(.., flags)| flags.pacing)
        };
        assert_eq!(pacing(&[]), Some(None));
        assert_eq!(pacing(&["--begin-frames"]), Some(Some(Pacing::Shell)));
        assert_eq!(
            pacing(&["--measure", "scroll", "--no-begin-frames"]),
            Some(Some(Pacing::CefTimer))
        );
        assert_eq!(
            pacing(&["--smoke", "page", "--begin-frames"]),
            Some(Some(Pacing::Shell))
        );
        assert_eq!(pacing(&["--begin-frames", "--no-begin-frames"]), None);
    }

    #[test]
    fn page_layer_flag_rows() {
        let flags = |args: &[&str]| {
            parse_with_shot(std::iter::once("wubble").chain(args.iter().copied()))
                .ok()
                .map(|(.., flags)| flags)
        };
        let layer = |mode| RunFlags {
            page_layer: mode,
            ..RunFlags::default()
        };
        #[rustfmt::skip]
        let rows: &[(&str, &[&str], Option<RunFlags>)] = &[
            ("page_layer_defaults_off", &[], Some(layer(PageLayerMode::Off))),
            ("off", &["--page-layer", "off"], Some(layer(PageLayerMode::Off))),
            ("fullscreen", &["--page-layer=fullscreen"], Some(layer(PageLayerMode::Fullscreen))),
            ("on is not yet a mode", &["--page-layer", "on"], None),
            ("an unknown mode", &["--page-layer", "x"], None),
            ("with the pacing", &["--begin-frames", "--page-layer", "fullscreen", "--smoke", "page"],
                Some(RunFlags { pacing: Some(Pacing::Shell), page_layer: PageLayerMode::Fullscreen })),
        ];
        for &(name, args, want) in rows {
            assert_eq!(flags(args), want, "{name}");
        }
    }

    #[test]
    fn measure_page_is_for_idle() {
        let parsed = parse([
            "wubble",
            "--measure",
            "idle",
            "--measure-page",
            "wubble://start",
        ]);
        assert!(matches!(
            parsed,
            Ok(Command::Measure(Measure { page: Some(ref p), .. })) if p == "wubble://start"
        ));
        assert!(
            parse([
                "wubble",
                "--measure",
                "video",
                "--measure-page",
                "wubble://start"
            ])
            .is_err()
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
