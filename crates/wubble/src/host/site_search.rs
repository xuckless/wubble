//! The host's half of site search discovery (`wubble_core::search::Discovery`): when a page has
//! loaded and `search.discover_sites` is on, a page script reads the search description it
//! links (`<link rel="search" type="application/opensearchdescription+xml">`); the
//! description is fetched in the tab's network session (`wubble_engine::fetch`, without
//! cookies), and a site's engine it describes is added to `search.discovered`, inactive.
//! With the setting off (the default) no script runs and nothing is fetched.

use std::collections::{BTreeMap, HashMap};

use wubble_core::apply;
use wubble_core::search::{self, Discovery, DiscoveryContext, DiscoveryEffect, DiscoveryEvent};
use wubble_core::settings as defs;
use wubble_core::tab::TabId;
use wubble_engine::fetch::{FetchEvent, FetchId, FetchSpec, Fetcher};
use wubble_engine::nav::WebEvent;
use wubble_engine::page::PageEvent;

use super::Host;
use crate::pagescripts;

/// The largest description read; a longer answer is dropped (they are a few hundred bytes).
const MAX_DESCRIPTION: usize = 64 * 1024;

/// One description being fetched: its address and the bytes so far.
#[derive(Debug)]
struct Fetching {
    osdd: String,
    body: Vec<u8>,
}

/// What the discovery holds while the app runs: the machine, and its edge (the page scripts
/// asked, the fetches out).
#[derive(Debug, Default)]
pub(super) struct SiteSearch {
    /// `settings::SEARCH_DISCOVER_SITES`.
    enabled: bool,
    machine: Discovery,
    /// The link scripts in flight, by eval id.
    probes: BTreeMap<u64, TabId>,
    fetcher: Fetcher,
    fetches: HashMap<FetchId, Fetching>,
}

impl Host {
    /// The setting changed (or was read at start). Off: what is out is stopped.
    pub(super) fn site_search_enabled(&mut self, on: bool) {
        self.site_search.enabled = on;
        if !on {
            self.site_search.probes.clear();
            self.site_search.fetcher.stop_all();
            let out: Vec<String> = self
                .site_search
                .fetches
                .drain()
                .map(|(_, f)| f.osdd)
                .collect();
            for osdd in out {
                self.site_search_event(DiscoveryEvent::Fetched { osdd, body: None });
            }
        }
    }

    /// A page finished loading: with discovery on, ask it for its search description.
    pub(super) fn site_search_web(&mut self, tab: TabId, event: &WebEvent) {
        if !self.site_search.enabled || !matches!(event, WebEvent::LoadingChanged { loading: false, .. }) {
            return;
        }
        let page = self.committed.get(&tab).cloned().unwrap_or_default();
        if search::generate_keyword(&page).is_none() {
            return;
        }
        if let Some(id) = self
            .tab_view(tab)
            .and_then(|v| v.eval(pagescripts::OPENSEARCH_LINK))
        {
            self.site_search.probes.insert(id.0, tab);
        }
    }

    /// The link script's answer; `true` when the event was one.
    pub(super) fn site_search_answer(&mut self, tab: TabId, event: &PageEvent) -> bool {
        let PageEvent::EvalResult { id, result } = event else {
            return false;
        };
        if self.site_search.probes.remove(&id.0).is_none() {
            return false;
        }
        let osdd = result.as_deref().ok().and_then(search::osdd_link);
        let page = self.committed.get(&tab).cloned();
        if let (Some(osdd), Some(page)) = (osdd, page) {
            self.site_search_event(DiscoveryEvent::Found { tab, page, osdd });
        }
        true
    }

    /// The fetches' bytes and ends, on every pump with CEF work.
    pub(super) fn site_search_pump(&mut self) {
        if self.site_search.fetcher.is_idle() {
            return;
        }
        for event in self.site_search.fetcher.take_events() {
            match event {
                FetchEvent::Data { id, bytes } => {
                    let Some(f) = self.site_search.fetches.get_mut(&id) else {
                        continue;
                    };
                    f.body.extend_from_slice(&bytes);
                    if f.body.len() > MAX_DESCRIPTION {
                        self.site_search.fetcher.cancel(id);
                    }
                }
                FetchEvent::Done { id, result } => {
                    let Some(f) = self.site_search.fetches.remove(&id) else {
                        continue;
                    };
                    let body = result
                        .ok()
                        .filter(|_| f.body.len() <= MAX_DESCRIPTION)
                        .and_then(|_| String::from_utf8(f.body).ok());
                    self.site_search_event(DiscoveryEvent::Fetched { osdd: f.osdd, body });
                }
            }
        }
    }

    /// Shutdown, before CEF goes: the fetches are stopped and released.
    pub(super) fn site_search_shutdown(&mut self) {
        self.site_search.probes.clear();
        self.site_search.fetches.clear();
        self.site_search.fetcher.stop_all();
    }

    fn site_search_event(&mut self, event: DiscoveryEvent) {
        let cx = DiscoveryContext {
            enabled: self.site_search.enabled,
            engines: &self.search,
        };
        let (next, effects) =
            search::transition(std::mem::take(&mut self.site_search.machine), event, &cx);
        self.site_search.machine = next;
        for effect in effects {
            match effect {
                DiscoveryEffect::Fetch { tab, osdd } => self.site_search_fetch(tab, osdd),
                DiscoveryEffect::Add(entry) => {
                    let key = defs::SEARCH_DISCOVERED.key;
                    if let Ok(text) =
                        apply::put_entry(key, &self.settings.stored, None, &entry.text())
                    {
                        self.write_setting(key, &text);
                    }
                }
            }
        }
    }

    /// Fetches a description in `tab`'s request context, without its cookies; a refused start
    /// answers at once as a failure, so the machine moves on.
    fn site_search_fetch(&mut self, tab: TabId, osdd: String) {
        let started = match self.tab_view(tab).map(|v| v.browser_id()) {
            Some(browser_id) => self.site_search.fetcher.start(&FetchSpec {
                browser_id,
                url: osdd.clone(),
                cached: true,
                ..FetchSpec::default()
            }),
            None => Err("the page is gone".to_owned()),
        };
        match started {
            Ok(id) => {
                self.site_search.fetches.insert(
                    id,
                    Fetching {
                        osdd,
                        body: Vec::new(),
                    },
                );
            }
            Err(e) => {
                eprintln!("wubble: site search description {osdd}: {e}");
                self.site_search_event(DiscoveryEvent::Fetched { osdd, body: None });
            }
        }
    }
}
