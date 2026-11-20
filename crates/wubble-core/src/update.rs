//! The updater: when wubble opens it asks GitHub Releases for the channel's manifest (one plain
//! GET of a static file, at most once per [`Policy::every_secs`]; Settings has the switch),
//! and a newer release for this platform is downloaded, checked against the signed manifest
//! and its own signature, unpacked beside the running version and made the one the next launch
//! runs. One lifecycle ([`State`]) changed only by [`transition`]; the effects are values the
//! binary executes (`crates/wubble/src/updater`).
//!
//! Inert, never leaving [`State::Inert`], while no key is compiled in ([`key::PUBLIC_KEY`] is
//! the placeholder), in a development build, or in an install wubble did not make itself
//! (a system package): those are updated elsewhere.
//!
//! Ported in outline from Zed's `auto_update` crate (zed-industries/zed, GPL-3.0: poll on start,
//! a status enum Idle / Checking / Downloading / Installing / Updated / Errored shown in the UI,
//! "restart to update") and Tauri's updater plugin (MIT OR Apache-2.0: the manifest and the
//! minisign signatures, `manifest`). No code copied.

pub mod install;
pub mod key;
pub mod manifest;

pub use semver::Version;

pub use manifest::{Artifact, Manifest, Offer, Refusal};

/// The rolling release holding the channel manifests (`<channel>.json` and `.minisig`).
pub const MANIFEST_BASE: &str = "https://github.com/xuckless/wubble/releases/download/updates/";

/// This build's version: the workspace's.
pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("the workspace version is semver")
}

/// This build's platform id, as the manifest and the archive names spell it.
pub const PLATFORM: Option<&str> = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("linux-x86_64")
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("macos-aarch64")
} else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    Some("macos-x86_64")
} else {
    None
};

/// Every platform id a release can carry.
pub const PLATFORMS: [&str; 3] = ["linux-x86_64", "macos-aarch64", "macos-x86_64"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// Pre-releases and releases.
    Preview,
    /// Releases only.
    Stable,
}

impl Channel {
    /// A preview build follows previews; a release build follows releases.
    pub fn of(v: &Version) -> Channel {
        if v.pre.is_empty() {
            Channel::Stable
        } else {
            Channel::Preview
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Channel::Preview => "preview",
            Channel::Stable => "stable",
        }
    }

    pub fn accepts(self, v: &Version) -> bool {
        self == Channel::Preview || v.pre.is_empty()
    }

    /// The manifest's URL under `base` (`MANIFEST_BASE`); its signature is at `+ ".minisig"`.
    pub fn manifest_url(self, base: &str) -> String {
        format!("{base}{}.json", self.id())
    }
}

/// When a check on open is due.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub every_secs: u64,
}

pub const POLICY: Policy = Policy {
    every_secs: 6 * 3600,
};

impl Policy {
    /// `last`: the last check, Unix seconds (0: never). A clock that went back counts as due.
    pub fn due(self, last: u64, now: u64) -> bool {
        last == 0 || now < last || now - last >= self.every_secs
    }
}

/// Why the updater does nothing in this build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inert {
    /// No signing key is compiled in.
    NoKey,
    /// Run from a cargo target directory.
    Development,
    /// A system package or a copy wubble does not manage.
    Elsewhere,
    /// No release is built for this OS and processor.
    NoPlatform,
}

/// What the lifecycle needs to judge a manifest; fixed for the run.
#[derive(Clone, Debug)]
pub struct Context {
    pub current: Version,
    pub platform: &'static str,
    pub channel: Channel,
    pub key: minisign_verify::PublicKey,
    /// The version that failed to start here (`install::LaunchRecord::bad`).
    pub skip: Option<Version>,
    pub policy: Policy,
    pub manifest_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Not checked in this run.
    Idle,
    Inert(Inert),
    Checking,
    UpToDate,
    /// Found, the download not started yet.
    Available(Offer),
    Downloading {
        offer: Offer,
        received: u64,
    },
    /// The download's size, hash and signature.
    Verifying(Offer),
    /// Unpacking and putting it beside the running version.
    Staging(Offer),
    /// Applies at the next launch.
    Ready(Version),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// wubble opened: `enabled` is the setting, `last` the last check (Unix seconds, 0 never).
    Opened {
        enabled: bool,
        last: u64,
        now: u64,
    },
    /// Settings' "Check now".
    CheckNow,
    /// The manifest and its signature's text, or why they could not be fetched.
    Manifest(Result<(Vec<u8>, String), String>),
    /// Bytes received so far.
    Progress(u64),
    Downloaded(Result<(), String>),
    Verified(Result<(), Refusal>),
    Staged(Result<(), String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// GET the manifest and `url + ".minisig"`.
    FetchManifest { url: String },
    /// Store the check's time (`update.last_check`).
    RecordCheck,
    /// To a file in the staging area; report [`Event::Progress`] and [`Event::Downloaded`].
    Download(Offer),
    /// [`manifest::verify_archive`] over the downloaded file.
    Verify(Offer),
    /// Unpack, check its release file, put it in place for the next launch.
    Stage(Offer),
    /// The toast: ready, restart to update.
    Notify(Version),
}

/// Where the lifecycle starts: inert when nothing could ever be installed.
pub fn initial(key: bool, install: &install::InstallKind, platform: Option<&str>) -> State {
    match (key, install, platform) {
        (_, install::InstallKind::Development, _) => State::Inert(Inert::Development),
        (_, install::InstallKind::Elsewhere, _) => State::Inert(Inert::Elsewhere),
        (false, _, _) => State::Inert(Inert::NoKey),
        (_, _, None) => State::Inert(Inert::NoPlatform),
        (true, install::InstallKind::Managed(_), Some(_)) => State::Idle,
    }
}

pub fn transition(state: State, event: Event, ctx: &Context) -> (State, Vec<Effect>) {
    use Effect::*;
    let check = || {
        (
            State::Checking,
            vec![
                FetchManifest {
                    url: ctx.manifest_url.clone(),
                },
                RecordCheck,
            ],
        )
    };
    match (state, event) {
        (s @ (State::Inert(_) | State::Ready(_)), _) => (s, vec![]),
        (State::Idle, Event::Opened { enabled, last, now }) => {
            if enabled && ctx.policy.due(last, now) {
                check()
            } else {
                (State::Idle, vec![])
            }
        }
        (State::Idle | State::UpToDate | State::Failed(_), Event::CheckNow) => check(),
        (State::Checking, Event::Manifest(Err(e))) => (State::Failed(e), vec![]),
        (State::Checking, Event::Manifest(Ok((body, sig)))) => {
            let chosen = manifest::verify_manifest(&body, &sig, &ctx.key).and_then(|m| {
                manifest::choose(
                    &m,
                    &ctx.current,
                    ctx.platform,
                    ctx.channel,
                    ctx.skip.as_ref(),
                )
            });
            match chosen {
                Ok(Some(offer)) => (State::Available(offer.clone()), vec![Download(offer)]),
                Ok(None) => (State::UpToDate, vec![]),
                Err(r) => (State::Failed(r.to_string()), vec![]),
            }
        }
        (State::Available(offer) | State::Downloading { offer, .. }, Event::Progress(received)) => {
            (State::Downloading { offer, received }, vec![])
        }
        (State::Available(offer) | State::Downloading { offer, .. }, Event::Downloaded(r)) => {
            match r {
                Ok(()) => (State::Verifying(offer.clone()), vec![Verify(offer)]),
                Err(e) => (State::Failed(e), vec![]),
            }
        }
        (State::Verifying(offer), Event::Verified(r)) => match r {
            Ok(()) => (State::Staging(offer.clone()), vec![Stage(offer)]),
            Err(r) => (State::Failed(r.to_string()), vec![]),
        },
        (State::Staging(offer), Event::Staged(r)) => match r {
            Ok(()) => (
                State::Ready(offer.version.clone()),
                vec![Notify(offer.version)],
            ),
            Err(e) => (State::Failed(e), vec![]),
        },
        // a late or repeated event: nothing changes
        (s, _) => (s, vec![]),
    }
}

/// What Settings shows of the lifecycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub status: String,
    /// "Check now" does something.
    pub can_check: bool,
    /// "Restart to update" shows.
    pub ready: bool,
    /// The whole group is informational: updates come from elsewhere.
    pub inert: bool,
}

pub fn view(state: &State) -> View {
    let status = match state {
        State::Idle => "Not checked since wubble opened".to_owned(),
        State::Inert(Inert::NoKey) => "Updates are not set up in this build".to_owned(),
        State::Inert(Inert::Development) => {
            "Development build: updates are managed elsewhere".to_owned()
        }
        State::Inert(Inert::Elsewhere) => {
            "Updates are managed elsewhere (this copy was not installed by wubble)".to_owned()
        }
        State::Inert(Inert::NoPlatform) => "No updates are built for this computer".to_owned(),
        State::Checking => "Checking for updates…".to_owned(),
        State::UpToDate => "wubble is up to date".to_owned(),
        State::Available(o) => format!("wubble {} is available: downloading", o.version),
        State::Downloading { offer, received } => {
            let total = offer.artifact.size.max(1);
            let percent = (received.saturating_mul(100) / total).min(100);
            format!("Downloading wubble {}: {percent} %", offer.version)
        }
        State::Verifying(o) => format!("Checking the download of wubble {}", o.version),
        State::Staging(o) => format!("Installing wubble {}", o.version),
        State::Ready(v) => ready_text(v),
        State::Failed(e) => format!("Update failed: {e}"),
    };
    View {
        status,
        can_check: matches!(state, State::Idle | State::UpToDate | State::Failed(_)),
        ready: matches!(state, State::Ready(_)),
        inert: matches!(state, State::Inert(_)),
    }
}

/// The toast and the status once an update is in place.
pub fn ready_text(v: &Version) -> String {
    format!("wubble {v} is ready: restart to update")
}

/// The toast's button.
pub const RESTART_ACTION: &str = "Restart";

#[cfg(test)]
mod tests {
    use super::install::{InstallKind, Layout, Os};
    use super::manifest::tests::{Signer, manifest_for, v};
    use super::*;

    const ARCHIVE: &[u8] = b"bundle bytes";

    fn ctx(signer: &Signer, current: &str) -> Context {
        Context {
            current: v(current),
            platform: "linux-x86_64",
            channel: Channel::of(&v(current)),
            key: signer.key(),
            skip: None,
            policy: POLICY,
            manifest_url: Channel::of(&v(current)).manifest_url(MANIFEST_BASE),
        }
    }

    fn signed(signer: &Signer, version: &str) -> Event {
        let text = manifest_for(signer, version, ARCHIVE)
            .to_json()
            .into_bytes();
        let sig = signer.sign(&text);
        Event::Manifest(Ok((text, sig)))
    }

    fn run(ctx: &Context, events: Vec<Event>) -> (State, Vec<Effect>) {
        let mut state = State::Idle;
        let mut all = vec![];
        for e in events {
            let (next, effects) = transition(state, e, ctx);
            state = next;
            all.extend(effects);
        }
        (state, all)
    }

    #[test]
    fn opening_checks_downloads_verifies_and_stages() {
        let s = Signer::new();
        let c = ctx(&s, "0.1.0-preview.1");
        let (state, effects) = run(
            &c,
            vec![
                Event::Opened {
                    enabled: true,
                    last: 0,
                    now: 100,
                },
                signed(&s, "0.1.0-preview.2"),
                Event::Progress(4),
                Event::Downloaded(Ok(())),
                Event::Verified(Ok(())),
                Event::Staged(Ok(())),
            ],
        );
        assert_eq!(state, State::Ready(v("0.1.0-preview.2")));
        let names: Vec<&str> = effects
            .iter()
            .map(|e| match e {
                Effect::FetchManifest { url } => {
                    assert_eq!(
                        url,
                        "https://github.com/xuckless/wubble/releases/download/updates/preview.json"
                    );
                    "fetch"
                }
                Effect::RecordCheck => "record",
                Effect::Download(_) => "download",
                Effect::Verify(_) => "verify",
                Effect::Stage(_) => "stage",
                Effect::Notify(_) => "notify",
            })
            .collect();
        assert_eq!(
            names,
            ["fetch", "record", "download", "verify", "stage", "notify"]
        );
        // Ready stays, whatever comes
        let (state, effects) = transition(state, Event::CheckNow, &c);
        assert!(matches!(state, State::Ready(_)) && effects.is_empty());
    }

    #[test]
    fn the_switch_and_the_policy_gate_the_check_on_open() {
        let s = Signer::new();
        let c = ctx(&s, "0.1.0");
        let opened = |enabled, last, now| Event::Opened { enabled, last, now };
        assert_eq!(run(&c, vec![opened(false, 0, 100)]).0, State::Idle);
        assert_eq!(run(&c, vec![opened(true, 100, 200)]).0, State::Idle);
        assert_eq!(
            run(&c, vec![opened(true, 100, 100 + POLICY.every_secs)]).0,
            State::Checking
        );
        // a clock set back is due
        assert_eq!(run(&c, vec![opened(true, 500, 100)]).0, State::Checking);
        // "Check now" ignores both
        assert_eq!(
            run(&c, vec![opened(false, 0, 1), Event::CheckNow]).0,
            State::Checking
        );
    }

    #[test]
    fn a_bad_manifest_or_download_fails_and_check_now_retries() {
        let s = Signer::new();
        let c = ctx(&s, "0.1.0");
        // signed by another key
        let (state, _) = run(&c, vec![Event::CheckNow, signed(&Signer::new(), "0.2.0")]);
        assert_eq!(state, State::Failed("signed with another key".into()));
        // a tampered archive is refused before staging
        let (state, effects) = run(
            &c,
            vec![
                Event::CheckNow,
                signed(&s, "0.2.0"),
                Event::Downloaded(Ok(())),
                Event::Verified(Err(Refusal::Tampered)),
            ],
        );
        assert!(matches!(state, State::Failed(_)));
        assert!(!effects.iter().any(|e| matches!(e, Effect::Stage(_))));
        let (state, _) = transition(state, Event::CheckNow, &c);
        assert_eq!(state, State::Checking);
        // offline
        let (state, _) = run(
            &c,
            vec![Event::CheckNow, Event::Manifest(Err("offline".into()))],
        );
        assert_eq!(view(&state).status, "Update failed: offline");
    }

    #[test]
    fn an_older_or_equal_release_is_up_to_date() {
        let s = Signer::new();
        let c = ctx(&s, "0.2.0");
        assert_eq!(
            run(&c, vec![Event::CheckNow, signed(&s, "0.1.0")]).0,
            State::UpToDate
        );
        assert_eq!(
            run(&c, vec![Event::CheckNow, signed(&s, "0.2.0")]).0,
            State::UpToDate
        );
    }

    #[test]
    fn inert_installs_never_check() {
        let managed = InstallKind::Managed(Layout {
            os: Os::Linux,
            root: "/r".into(),
            bundle: "/r/versions/0.1.0".into(),
        });
        assert_eq!(initial(true, &managed, Some("linux-x86_64")), State::Idle);
        assert_eq!(
            initial(false, &managed, Some("linux-x86_64")),
            State::Inert(Inert::NoKey)
        );
        assert_eq!(
            initial(true, &InstallKind::Development, Some("linux-x86_64")),
            State::Inert(Inert::Development)
        );
        assert_eq!(
            initial(true, &InstallKind::Elsewhere, Some("linux-x86_64")),
            State::Inert(Inert::Elsewhere)
        );
        assert_eq!(
            initial(true, &managed, None),
            State::Inert(Inert::NoPlatform)
        );
        let s = Signer::new();
        let c = ctx(&s, "0.1.0");
        let inert = State::Inert(Inert::NoKey);
        for e in [
            Event::CheckNow,
            Event::Opened {
                enabled: true,
                last: 0,
                now: 1,
            },
        ] {
            assert_eq!(transition(inert.clone(), e, &c), (inert.clone(), vec![]));
        }
        let shown = view(&State::Inert(Inert::Development));
        assert!(shown.inert && !shown.can_check && shown.status.contains("managed elsewhere"));
    }

    #[test]
    fn channels_follow_the_build() {
        assert_eq!(Channel::of(&v("0.1.0-preview.3")), Channel::Preview);
        assert_eq!(Channel::of(&v("1.0.0")), Channel::Stable);
        assert!(Channel::Stable.accepts(&v("1.0.1")));
        assert!(!Channel::Stable.accepts(&v("1.1.0-preview.1")));
        assert!(Channel::Preview.accepts(&v("1.0.1")));
    }

    #[test]
    fn the_view_says_where_it_is() {
        let s = Signer::new();
        let offer = Offer {
            version: v("0.2.0"),
            artifact: manifest_for(&s, "0.2.0", &[0; 200]).platforms["linux-x86_64"].clone(),
        };
        let d = view(&State::Downloading {
            offer,
            received: 50,
        });
        assert_eq!(d.status, "Downloading wubble 0.2.0: 25 %");
        assert!(!d.can_check && !d.ready);
        let r = view(&State::Ready(v("0.2.0")));
        assert!(r.ready);
        assert_eq!(r.status, "wubble 0.2.0 is ready: restart to update");
        assert!(view(&State::UpToDate).can_check);
    }

    #[test]
    fn the_workspace_version_is_semver() {
        let _ = current_version();
    }
}
