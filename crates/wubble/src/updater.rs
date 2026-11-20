//! The updater's edge: runs `wubble_core::update`'s lifecycle and executes its effects. Each
//! effect that touches the network or the disk runs on a thread of its own and posts its
//! [`Event`] back (the browser: onto the UI thread, through `post`); the host keeps the
//! [`Updater`] and shows its [`View`] in Settings.
//!
//! The trust key is the compiled-in one (`update::key::PUBLIC_KEY`): [`Updater::production`]
//! reads nothing else, no flag, environment variable or file can change it, and while it is the
//! placeholder the updater stays inert. Only `--smoke update` builds an [`Updater`] with a key
//! of its own, in its own process, through [`Updater::with`].

pub mod install;
pub mod net;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use wubble_core::update::{self, Context, Effect, Event, State, View, install::InstallKind};

use install::Install;
use net::Net;

type Post = Arc<dyn Fn(Event) + Send + Sync>;

/// What the host does with the lifecycle's effects that are its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostEffect {
    /// Store the check's time in `update.last_check`.
    RecordCheck,
    /// The toast: `update::ready_text`, with the restart button.
    Notify(update::Version),
}

pub struct Updater {
    state: State,
    /// `None` while inert.
    ctx: Option<Context>,
    install: Option<Arc<dyn Install>>,
    net: Net,
    post: Post,
}

impl Updater {
    /// The browser's: this install, the compiled-in key, GitHub's manifest.
    pub fn production(post: impl Fn(Event) + Send + Sync + 'static) -> Updater {
        let (kind, install) = install::detect();
        let key = update::key::trusted();
        let state = update::initial(key.is_some(), &kind, update::PLATFORM);
        let install: Option<Arc<dyn Install>> = install.map(Arc::from);
        let ctx = match (&state, key, update::PLATFORM, &install) {
            (State::Idle, Some(key), Some(platform), Some(install)) => {
                let current = update::current_version();
                let channel = update::Channel::of(&current);
                Some(Context {
                    skip: install::read_record(install.layout()).bad,
                    current,
                    platform,
                    channel,
                    key,
                    policy: update::POLICY,
                    manifest_url: channel.manifest_url(update::MANIFEST_BASE),
                })
            }
            _ => None,
        };
        if let InstallKind::Managed(l) = &kind {
            eprintln!(
                "wubble: self-managed install at {} ({:?})",
                l.root.display(),
                state
            );
        }
        Updater {
            state,
            ctx,
            install,
            net: Net::PRODUCTION,
            post: Arc::new(post),
        }
    }

    /// `--smoke update`'s: its own key, install root and loopback server.
    pub fn with(
        ctx: Context,
        install: Arc<dyn Install>,
        net: Net,
        post: impl Fn(Event) + Send + Sync + 'static,
    ) -> Updater {
        Updater {
            state: State::Idle,
            ctx: Some(ctx),
            install: Some(install),
            net,
            post: Arc::new(post),
        }
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn view(&self) -> View {
        update::view(&self.state)
    }

    /// The program "Restart to update" starts, once an update is ready.
    pub fn relaunch_exe(&self) -> Option<PathBuf> {
        matches!(self.state, State::Ready(_))
            .then(|| self.install.as_ref().map(|i| i.layout().relaunch_exe()))
            .flatten()
    }

    /// One event through the lifecycle; the effects that are the host's come back.
    pub fn event(&mut self, event: Event) -> Vec<HostEffect> {
        let Some(ctx) = &self.ctx else {
            return vec![];
        };
        let state = std::mem::replace(&mut self.state, State::Idle);
        let (next, effects) = update::transition(state, event, ctx);
        self.state = next;
        let mut host = vec![];
        for effect in effects {
            match effect {
                Effect::RecordCheck => host.push(HostEffect::RecordCheck),
                Effect::Notify(v) => host.push(HostEffect::Notify(v)),
                other => self.execute(other),
            }
        }
        host
    }

    fn archive_path(&self, offer: &update::Offer) -> Option<PathBuf> {
        let install = self.install.as_ref()?;
        Some(
            install
                .layout()
                .staging()
                .join(format!("wubble-{}.download", offer.version)),
        )
    }

    fn execute(&self, effect: Effect) {
        let post = self.post.clone();
        let net = self.net;
        let (Some(ctx), Some(install), Some(archive)) = (
            self.ctx.clone(),
            self.install.clone(),
            match &effect {
                Effect::Download(o) | Effect::Verify(o) | Effect::Stage(o) => self.archive_path(o),
                _ => Some(PathBuf::new()),
            },
        ) else {
            return;
        };
        let job = move || match effect {
            Effect::FetchManifest { url } => post(Event::Manifest(net.manifest(&url))),
            Effect::Download(offer) => {
                let result = std::fs::create_dir_all(install.layout().staging())
                    .map_err(|e| e.to_string())
                    .and_then(|()| {
                        net.download(&offer.artifact.url, offer.artifact.size, &archive, |n| {
                            post(Event::Progress(n))
                        })
                    });
                if result.is_err() {
                    let _ = std::fs::remove_file(&archive);
                }
                post(Event::Downloaded(result));
            }
            Effect::Verify(offer) => {
                let result = std::fs::File::open(&archive)
                    .map_err(|e| update::Refusal::Malformed(e.to_string()))
                    .and_then(|f| {
                        update::manifest::verify_archive(
                            std::io::BufReader::new(f),
                            &offer.artifact,
                            &ctx.key,
                        )
                    });
                if result.is_err() {
                    // never kept, never unpacked
                    let _ = std::fs::remove_file(&archive);
                }
                post(Event::Verified(result));
            }
            Effect::Stage(offer) => post(Event::Staged(install::stage(
                install.as_ref(),
                &archive,
                &offer.version,
                ctx.platform,
                &ctx.current,
            ))),
            Effect::RecordCheck | Effect::Notify(_) => {}
        };
        if let Err(e) = std::thread::Builder::new()
            .name("wubble-update".into())
            .spawn(job)
        {
            eprintln!("wubble: cannot start the updater's thread: {e}");
        }
    }

    /// This run has been healthy long enough (`HEALTHY_AFTER_SECS`): the launch record
    /// keeps it, older versions go.
    pub fn healthy(&self) {
        if let Some(install) = self.install.clone() {
            let _ = std::thread::Builder::new()
                .name("wubble-update".into())
                .spawn(move || install::on_healthy(install.as_ref()));
        }
    }
}

/// The first thing a browser start does after it is the profile's one instance: a staged
/// update applies, or a failing version rolls back (`install::on_launch`).
pub fn on_launch() {
    if let (_, Some(install)) = install::detect() {
        install::on_launch(install.as_ref());
    }
}

/// Set by "Restart to update"; run once the browser has shut down.
static RELAUNCH: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn request_relaunch(exe: PathBuf) {
    *RELAUNCH.lock().unwrap_or_else(|e| e.into_inner()) = Some(exe);
}

/// After the shutdown: starts the relaunch program once this process has exited (the profile's
/// single-instance socket is free then). Only ever a program of the install itself.
pub fn relaunch_if_requested() {
    let Some(exe) = RELAUNCH.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return;
    };
    let pid = std::process::id().to_string();
    let started = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"while kill -0 "$0" 2>/dev/null; do sleep 0.2; done; exec "$1""#)
        .arg(&pid)
        .arg(&exe)
        .stdin(std::process::Stdio::null())
        .spawn();
    if let Err(e) = started {
        eprintln!("wubble: cannot restart {}: {e}", exe.display());
    }
}
