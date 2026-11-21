//! Hosting the MCP server, switched live (`wubble_core::agent::hosting`): its effects run here.
//! The socket and its listener thread are `mcp.rs`'s; the manifest is the file a relay finds
//! the browser by (`wubble_core::agent::manifest`).

use std::path::PathBuf;

use wubble_core::agent::hosting::{self, Effect, Event, Hosting};
use wubble_core::agent::manifest::{self, Manifest};

use super::Host;
use crate::mcp;

/// Where this browser hosts: its MCP socket, its manifest, its profile and binary.
#[derive(Clone, Debug)]
pub struct McpPlace {
    pub socket: PathBuf,
    /// In the user's private root (`mcp::manifest_dir`).
    pub manifest: PathBuf,
    pub profile: PathBuf,
    pub default_profile: bool,
    /// This browser's binary (the relay's command).
    pub exe: PathBuf,
    /// Where the agent apps' files are looked for: the home directory, or a scratch one in a
    /// smoke's profile (a check never writes the owner's apps); `None` when it could not be
    /// found: then nothing is read or written.
    pub home: Option<PathBuf>,
}

/// The hosting state and what runs it.
#[derive(Default)]
pub(crate) struct McpHosting {
    pub(crate) state: Hosting,
    /// Known once the window is up (`Host::set_mcp`); the setting replayed before waits for it.
    pub(super) place: Option<McpPlace>,
    wanted: bool,
    hosted: Option<mcp::Hosted>,
    /// Why an agent app was not written, by its id (shown on its row until the next try).
    pub(super) notes: std::collections::HashMap<&'static str, String>,
}

impl Host {
    /// Where this browser hosts; the stored switch takes effect now, as a start (`Resume`:
    /// wubble's own entries kept current, none added).
    pub fn set_mcp(&mut self, place: McpPlace) {
        self.hosting.place = Some(place);
        if self.hosting.wanted {
            self.mcp_hosting(Event::Resume);
        }
    }

    /// The switch, as the setting says (`Applied::McpHosting`): replayed at start before the
    /// place is known (only remembered then), or the owner's press.
    pub(super) fn mcp_switch(&mut self, on: bool) {
        self.hosting.wanted = on;
        if self.hosting.place.is_none() {
            return;
        }
        self.mcp_hosting(if on { Event::TurnOn } else { Event::TurnOff });
    }

    pub(super) fn mcp_serving(&self) -> bool {
        self.hosting.state.serving()
    }

    /// One event of hosting, through the core machine; its effects run here.
    pub(super) fn mcp_hosting(&mut self, event: Event) {
        let state = std::mem::take(&mut self.hosting.state);
        let (state, effects) = hosting::transition(state, event);
        self.hosting.state = state;
        for effect in effects {
            self.mcp_effect(effect);
        }
        self.render_settings();
    }

    fn mcp_effect(&mut self, effect: Effect) {
        let Some(place) = self.hosting.place.clone() else {
            return;
        };
        match effect {
            Effect::Bind => {
                let started = mcp::Hosted::start(&place.socket, |event| {
                    let _ = slint::invoke_from_event_loop(move || {
                        crate::with_host(|host| host.mcp_event(event))
                    });
                });
                match started {
                    Ok(hosted) => {
                        self.hosting.hosted = Some(hosted);
                        self.mcp_hosting(Event::Bound);
                    }
                    Err(e) => {
                        eprintln!("wubble: no MCP server: {}: {e}", place.socket.display());
                        self.mcp_hosting(Event::BindFailed(e.to_string()));
                    }
                }
            }
            Effect::Unbind => {
                if let Some(hosted) = self.hosting.hosted.take() {
                    hosted.stop();
                }
            }
            Effect::StopSessions => self.agents_stop(),
            Effect::WriteManifest => {
                let profile = place.profile.to_string_lossy().into_owned();
                let m = Manifest {
                    pid: std::process::id(),
                    command: manifest::command(
                        &place.exe.to_string_lossy(),
                        &profile,
                        place.default_profile,
                    ),
                    profile,
                    default_profile: place.default_profile,
                    socket: place.socket.to_string_lossy().into_owned(),
                    version: env!("CARGO_PKG_VERSION").into(),
                };
                if let Err(e) = mcp::write_manifest(&place.manifest, &m) {
                    eprintln!(
                        "wubble: no MCP manifest at {}: {e}",
                        place.manifest.display()
                    );
                }
            }
            Effect::RemoveManifest => {
                let _ = std::fs::remove_file(&place.manifest);
            }
            // the agent apps' configuration (`host/harness.rs`)
            Effect::Register => self.mcp_register(),
            Effect::Refresh => self.mcp_refresh(),
            Effect::Unregister => self.mcp_unregister(),
        }
    }

    /// The browser quits: sessions end, nothing is hosted, the files go.
    pub(super) fn mcp_quit(&mut self) {
        self.mcp_hosting(Event::Quit);
    }

    /// `--smoke mcp`: the files hosting keeps while on.
    pub(crate) fn mcp_files(&self) -> Option<(PathBuf, PathBuf)> {
        let p = self.hosting.place.as_ref()?;
        Some((p.socket.clone(), p.manifest.clone()))
    }
}
