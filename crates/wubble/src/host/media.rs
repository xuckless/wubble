//! The host's half of the media downloader: what each tab's page plays (`TabMedia`, fed by
//! the engine's response hook and the YouTube lookup), the pill's glyph and menu, and the
//! download jobs (`wubble_core::media::job`), whose fetches go through the engine's
//! [`Fetcher`] (the tab's network session) and whose bytes go to the engine's [`Writer`]. A job
//! is one row of the downloads list: the list's buttons arrive here as job events
//! (`media_list_effect`), the job's states go back as list events.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use slint::ComponentHandle;
use wubble_core::apply::MediaSettings;
use wubble_core::download::{self, DownloadEffect, DownloadEvent};
use wubble_core::media::detect::{
    self, Candidate, Detected, Found, Kind, MediaEffect, MediaEvent, TabMedia,
};
use wubble_core::media::job::{self, Job, JobEffect, JobEvent};
use wubble_core::media::select::{self, Selection};
use wubble_core::media::{
    Container, Plan, Refusal, Resolution, Track, Variant, dash, hls, name, youtube,
};
use wubble_core::space::ContainerId;
use wubble_core::tab::TabId;
use wubble_engine::fetch::{FetchEvent, FetchId, FetchSpec, Fetcher};
use wubble_engine::filter::{ResponseInfo, ResponseObserver};
use wubble_engine::save::{Join, WriteCmd, WriteEvent, Writer};

use super::Host;
use crate::media_smoke;
use crate::menu::MediaRow;

/// Largest manifest, key or player answer read into memory.
const MAX_TEXT: usize = 4 << 20;

/// The response hook's half, on CEF's IO thread: classifies and queues; the host drains it.
#[derive(Debug, Default)]
pub struct MediaObserver {
    enabled: AtomicBool,
    min_bytes: AtomicU64,
    seen: Mutex<Vec<(i32, Candidate)>>,
}

impl MediaObserver {
    pub fn new() -> Arc<Self> {
        let defaults = MediaSettings::default();
        let o = Self::default();
        o.enabled.store(defaults.enabled, Ordering::Relaxed);
        o.min_bytes.store(defaults.min_bytes, Ordering::Relaxed);
        Arc::new(o)
    }

    fn set(&self, settings: &MediaSettings) {
        self.enabled.store(settings.enabled, Ordering::Relaxed);
        self.min_bytes.store(settings.min_bytes, Ordering::Relaxed);
    }

    fn take(&self) -> Vec<(i32, Candidate)> {
        std::mem::take(&mut *self.seen.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl ResponseObserver for MediaObserver {
    fn seen(&self, r: &ResponseInfo<'_>) {
        if !self.enabled.load(Ordering::Relaxed) || r.browser_id == 0 {
            return;
        }
        let response = detect::Response {
            url: r.url,
            mime: r.mime,
            status: r.status,
            content_length: r.content_length,
            content_range: r.content_range,
            accepts_ranges: r.accepts_ranges,
        };
        if let Some(c) = detect::classify(&response, self.min_bytes.load(Ordering::Relaxed)) {
            self.seen
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((r.browser_id, c));
        }
    }
}

/// What a running request is for.
#[derive(Debug)]
enum Purpose {
    /// A detected stream's manifest (or the YouTube player answer).
    Resolve {
        tab: TabId,
        item: u32,
        kind: Kind,
        url: String,
        body: Vec<u8>,
    },
    /// One variant's media playlist of a resolved HLS master, read for DRM keys that only
    /// the variants carry, so the menu shows the refusal before offering anything.
    Probe {
        tab: TabId,
        item: u32,
        url: String,
        resolution: Resolution,
        body: Vec<u8>,
    },
    /// A job track's HLS media playlist.
    Playlist {
        job: i64,
        track: usize,
        url: String,
        body: Vec<u8>,
    },
    /// A job's AES-128 key.
    Key {
        job: i64,
        uri: String,
        body: Vec<u8>,
    },
    Segment {
        job: i64,
        track: usize,
        index: usize,
    },
}

impl Purpose {
    fn job(&self) -> Option<i64> {
        match self {
            Purpose::Resolve { .. } | Purpose::Probe { .. } => None,
            Purpose::Playlist { job, .. }
            | Purpose::Key { job, .. }
            | Purpose::Segment { job, .. } => Some(*job),
        }
    }
}

/// Request headers, name and value.
type Headers = Vec<(String, String)>;

/// One track of a job.
#[derive(Debug)]
struct TrackRun {
    variant: Variant,
    plan: Option<Plan>,
    part: PathBuf,
}

/// One download, from the choice to the saved file.
#[derive(Debug)]
struct Run {
    job: Job,
    /// The tab it was started from: its session fetches, while it lives.
    tab: TabId,
    /// The page the stream plays on (the fetches' referrer).
    page: String,
    title: String,
    note: String,
    tracks: Vec<TrackRun>,
    /// Where the joined file goes (its extension may change once the tracks are known).
    output: PathBuf,
    keys: HashMap<String, Vec<u8>>,
    /// Bytes fetched per segment (the list's progress).
    fetched: BTreeMap<(usize, usize), u64>,
    /// The known size of the whole download.
    /// That tab's container: the session a fetch keeps once the tab is gone.
    container: ContainerId,
    size: Option<u64>,
    /// What the join saved (deleted again when the job was cancelled while joining).
    saved: Vec<PathBuf>,
}

impl Run {
    fn received(&self) -> u64 {
        self.fetched.values().sum()
    }
}

/// The downloader's state, one owner.
#[derive(Debug)]
pub(super) struct Media {
    pub(super) observer: Arc<MediaObserver>,
    settings: MediaSettings,
    tabs: BTreeMap<TabId, TabMedia>,
    fetcher: Fetcher,
    writer: Option<Writer>,
    fetches: HashMap<FetchId, Purpose>,
    /// By downloads-list entry id.
    runs: BTreeMap<i64, Run>,
}

impl Media {
    pub fn new(observer: Arc<MediaObserver>) -> Self {
        let writer = Writer::start(|| {
            let _ = slint::invoke_from_event_loop(|| crate::with_host(|host| host.media_pump()));
        })
        .map_err(|e| eprintln!("wubble: no media writer thread: {e}"))
        .ok();
        Self {
            observer,
            settings: MediaSettings::default(),
            tabs: BTreeMap::new(),
            fetcher: Fetcher::default(),
            writer,
            fetches: HashMap::new(),
            runs: BTreeMap::new(),
        }
    }

    pub fn owns(&self, entry: i64) -> bool {
        self.runs.contains_key(&entry)
    }

    pub fn has_media(&self, tab: TabId) -> bool {
        self.settings.enabled && self.tabs.get(&tab).is_some_and(TabMedia::has_media)
    }

    fn write(&self, cmd: WriteCmd) {
        if let Some(w) = &self.writer {
            w.send(cmd);
        }
    }

    /// Every output path a job holds (reserved names).
    fn reserved(&self, path: &Path) -> bool {
        self.runs
            .values()
            .any(|r| r.output == path || r.tracks.iter().any(|t| t.part == path))
    }
}

/// The URL without its fragment.
fn page_key(url: &str) -> &str {
    url.split('#').next().unwrap_or_default()
}

impl Host {
    pub(super) fn apply_media_settings(&mut self, settings: MediaSettings) {
        self.media.observer.set(&settings);
        let youtube_off = !settings.youtube && self.media.settings.youtube;
        let save = self.video_save_now();
        self.media.settings = settings;
        if youtube_off {
            for list in self.media.tabs.values_mut() {
                list.items.retain(|i| i.kind != Kind::YouTube);
            }
        }
        if let Some(window) = self.w.window.upgrade() {
            self.render_pill(&window);
        }
        self.video_save_redraw(save);
    }

    /// The main frame of `tab` committed `url` (before `committed` moves on): another page
    /// clears the tab's list; a YouTube video is looked up when the setting allows.
    pub(super) fn media_navigated(&mut self, tab: TabId, url: &str) {
        let same = self
            .committed
            .get(&tab)
            .is_some_and(|old| page_key(old) == page_key(url));
        if !same {
            self.media_update(tab, MediaEvent::Navigated);
        }
        let s = &self.media.settings;
        if s.enabled && s.youtube && youtube::video_id(url).is_some() {
            let candidate = Candidate {
                url: page_key(url).to_owned(),
                kind: Kind::YouTube,
                track: Track::Muxed,
                container: Container::Mp4,
                size: None,
                ranges: false,
            };
            self.media_update(tab, MediaEvent::Seen(candidate));
        }
    }

    /// The tab closed: its list goes (its jobs keep running).
    pub(super) fn media_tab_closed(&mut self, tab: TabId) {
        self.media.tabs.remove(&tab);
    }

    fn media_update(&mut self, tab: TabId, event: MediaEvent) {
        let save = self.video_save_now();
        let list = self.media.tabs.remove(&tab).unwrap_or_default();
        let before = list.has_media();
        let (list, effects) = list.update(event);
        let after = list.has_media();
        self.media.tabs.insert(tab, list);
        for MediaEffect::Resolve { id, url, kind } in effects {
            self.media_resolve(tab, id, url, kind);
        }
        if before != after
            && let Some(window) = self.w.window.upgrade()
        {
            self.render_pill(&window);
        }
        self.video_save_redraw(save);
    }

    /// The tab's view, else a live page of the same container (the tab closed or went to
    /// sleep): whose session a fetch uses. Never a page of another container: its cookies are
    /// not the video's.
    fn fetch_browser(&mut self, tab: TabId, container: ContainerId) -> Option<i32> {
        if let Some(view) = self.tab_view(tab) {
            return Some(view.browser_id());
        }
        let same = self
            .views
            .keys()
            .copied()
            .find(|t| self.model.tabs.get(t).is_some_and(|m| m.container == container))?;
        self.tab_view(same).map(|v| v.browser_id())
    }

    fn media_fetch(&mut self, tab: TabId, spec: FetchSpec, purpose: Purpose) -> Result<(), String> {
        let container = purpose
            .job()
            .and_then(|j| self.media.runs.get(&j))
            .map(|r| r.container)
            .or_else(|| self.model.tabs.get(&tab).map(|t| t.container))
            .unwrap_or(ContainerId::DEFAULT);
        let browser_id = self.fetch_browser(tab, container).ok_or(
            "the page it came from is closed: open that site's page again, then resume",
        )?;
        let id = self
            .media
            .fetcher
            .start(&FetchSpec { browser_id, ..spec })?;
        self.media.fetches.insert(id, purpose);
        Ok(())
    }

    fn media_resolve(&mut self, tab: TabId, item: u32, url: String, kind: Kind) {
        let page = self.committed.get(&tab).cloned().unwrap_or_default();
        let spec = match kind {
            Kind::YouTube => {
                let id = youtube::video_id(&url).unwrap_or_default();
                let (body, headers) = youtube::player_request(&id);
                FetchSpec {
                    url: youtube::PLAYER_URL.into(),
                    headers,
                    post: Some(body.into_bytes()),
                    ..FetchSpec::default()
                }
            }
            // The manifest the page has just loaded: from the HTTP cache when it may be.
            _ => FetchSpec {
                url: url.clone(),
                referrer: page,
                credentials: true,
                cached: true,
                ..FetchSpec::default()
            },
        };
        let purpose = Purpose::Resolve {
            tab,
            item,
            kind,
            url,
            body: Vec::new(),
        };
        if let Err(e) = self.media_fetch(tab, spec, purpose) {
            let result = Err(Refusal::Failed(e));
            self.media_update(tab, MediaEvent::Resolved { id: item, result });
        }
    }

    /// Runs on every pump with CEF work, and when the writer reports: the hook's candidates,
    /// the fetches' bytes and ends, the writer's results.
    pub(crate) fn media_pump(&mut self) {
        for (browser_id, candidate) in self.media.observer.take() {
            let tab = self
                .views
                .iter()
                .find(|&(_, &v)| {
                    self.engine
                        .view(v)
                        .is_some_and(|view| view.browser_id() == browser_id)
                })
                .map(|(&t, _)| t);
            if let Some(tab) = tab {
                self.media_update(tab, MediaEvent::Seen(candidate));
            }
        }
        for event in self.media.fetcher.take_events() {
            match event {
                FetchEvent::Data { id, bytes } => self.media_data(id, bytes),
                FetchEvent::Done { id, result } => {
                    if let Some(purpose) = self.media.fetches.remove(&id) {
                        self.media_done(purpose, result.map(|_| ()));
                    }
                }
            }
        }
        let events = self
            .media
            .writer
            .as_ref()
            .map(Writer::take_events)
            .unwrap_or_default();
        for event in events {
            match event {
                WriteEvent::Joined { job, result } => self.media_joined(job, result),
                WriteEvent::Failed { job, error } => {
                    self.job_event(job, JobEvent::SegmentFailed(error))
                }
            }
        }
        self.media_smoke_step();
    }

    /// Shutdown, before CEF goes: every request of the downloader is stopped and released;
    /// unfinished downloads (which do not resume across a restart) delete their part files,
    /// and the writer finishes what it was sent.
    pub(super) fn media_shutdown(&mut self) {
        self.media.fetches.clear();
        self.media.fetcher.stop_all();
        let live: Vec<i64> = self
            .media
            .runs
            .iter()
            .filter(|(_, r)| r.job.is_live())
            .map(|(&id, _)| id)
            .collect();
        for id in live {
            self.job_effect(id, JobEffect::Discard);
        }
        if let Some(writer) = self.media.writer.take() {
            writer.finish();
        }
    }

    // ----- `--smoke media`

    fn media_smoke_step(&mut self) {
        let Some(state) = self.checks.media_smoke else {
            return;
        };
        let tab = self.model.visible_tab();
        let items: Vec<Detected> = tab
            .and_then(|t| self.media.tabs.get(&t))
            .map(|l| l.items.clone())
            .unwrap_or_default();
        let seen = media_smoke::Seen {
            file: items
                .iter()
                .any(|i| i.kind == Kind::File && matches!(i.found, Found::Ready(_))),
            hls_variants: items
                .iter()
                .filter(|i| i.kind == Kind::Hls)
                .map(|i| match &i.found {
                    Found::Ready(v) => v.len(),
                    _ => 0,
                })
                .sum(),
            protected: items
                .iter()
                .any(|i| i.found == Found::Refused(Refusal::Protected)),
            complete: self
                .media
                .runs
                .values()
                .filter(|r| r.job == Job::Done)
                .count(),
            failed: self.media.runs.values().find_map(|r| match &r.job {
                Job::Failed(e) | Job::Interrupted(_, e) => Some(e.clone()),
                _ => None,
            }),
        };
        let (state, actions) = media_smoke::next(state, &seen);
        self.checks.media_smoke = Some(state);
        for action in actions {
            match action {
                media_smoke::Action::SaveAll => {
                    let Some(tab) = tab else { continue };
                    let saves: Vec<(u32, usize)> = items
                        .iter()
                        .filter_map(|i| match &i.found {
                            Found::Ready(v) => Some((i.id, select::choices(v).len())),
                            _ => None,
                        })
                        .flat_map(|(id, n)| (0..n).map(move |c| (id, c)))
                        .collect();
                    for (item, choice) in saves {
                        self.save_media(tab, item, choice);
                    }
                }
                media_smoke::Action::Check => {
                    let saved: Result<Vec<Vec<u8>>, String> = self
                        .media
                        .runs
                        .values()
                        .filter(|r| r.job == Job::Done)
                        .map(|r| {
                            std::fs::read(&r.output)
                                .map_err(|e| format!("{}: {e}", r.output.display()))
                        })
                        .collect();
                    match saved.and_then(|s| media_smoke::check(&s)) {
                        Ok(()) => {
                            let names: Vec<String> = self
                                .media
                                .runs
                                .values()
                                .map(|r| r.output.display().to_string())
                                .collect();
                            println!("smoke: media saved {}", names.join(", "));
                            self.media_passed = true;
                            crate::quit();
                        }
                        Err(e) => self.fail(e),
                    }
                }
                media_smoke::Action::Fail(reason) => self.fail(reason),
            }
        }
    }

    fn media_data(&mut self, id: FetchId, bytes: Vec<u8>) {
        let Some(purpose) = self.media.fetches.get_mut(&id) else {
            return;
        };
        match purpose {
            Purpose::Resolve { body, .. }
            | Purpose::Probe { body, .. }
            | Purpose::Playlist { body, .. }
            | Purpose::Key { body, .. } => {
                if body.len() + bytes.len() > MAX_TEXT {
                    self.media.fetcher.cancel(id);
                } else {
                    body.extend_from_slice(&bytes);
                }
            }
            &mut Purpose::Segment { job, track, index } => {
                // The guard behind the manifest's markers: an MP4 stream whose start names
                // encryption is protected, and stops before anything is saved.
                let mp4 = self
                    .media
                    .runs
                    .get(&job)
                    .and_then(|r| r.tracks.get(track)?.plan.as_ref())
                    .is_some_and(|p| matches!(p.container, Container::Mp4 | Container::M4a));
                let fresh = self
                    .media
                    .runs
                    .get(&job)
                    .is_some_and(|r| !r.fetched.contains_key(&(track, index)));
                if index == 0 && fresh && mp4 && wubble_core::media::looks_encrypted(&bytes) {
                    self.job_event(job, JobEvent::Refused(Refusal::Protected.message()));
                    return;
                }
                let len = bytes.len() as u64;
                self.media.write(WriteCmd::Chunk {
                    job,
                    track,
                    index,
                    bytes,
                });
                let Some(run) = self.media.runs.get_mut(&job) else {
                    return;
                };
                *run.fetched.entry((track, index)).or_default() += len;
                self.media_progress(job);
            }
        }
    }

    /// The list row's bytes and expected total.
    fn media_progress(&mut self, job: i64) {
        let Some(run) = self.media.runs.get(&job) else {
            return;
        };
        let received = run.received();
        let total = match &run.job {
            Job::Fetching(p) | Job::Paused(p) | Job::Interrupted(p, _) => {
                job::estimate_total(run.size, received, p.fetched(), p.total())
            }
            _ => run.size,
        };
        self.apply_download(job, DownloadEvent::Progress { received, total });
    }

    fn media_done(&mut self, purpose: Purpose, result: Result<(), String>) {
        match purpose {
            Purpose::Resolve {
                tab,
                item,
                kind,
                url,
                body,
            } => {
                let text = String::from_utf8_lossy(&body);
                let result = match result {
                    Err(e) => Err(Refusal::Failed(e)),
                    Ok(()) => match kind {
                        Kind::Hls => hls::variants(&text, &url).map(|variants| Resolution {
                            variants,
                            title: None,
                        }),
                        Kind::Dash => dash::variants(&text, &url).map(|variants| Resolution {
                            variants,
                            title: None,
                        }),
                        Kind::YouTube => youtube::parse_player(&text),
                        Kind::File => Err(Refusal::Failed("not a manifest".into())),
                    },
                };
                if let Err(e) = &result {
                    eprintln!("wubble: media {url}: {}", e.message());
                }
                match result {
                    Ok(resolution) if kind == Kind::Hls => {
                        self.media_probe(tab, item, resolution);
                    }
                    result => self.media_update(tab, MediaEvent::Resolved { id: item, result }),
                }
            }
            Purpose::Probe {
                tab,
                item,
                url,
                resolution,
                body,
            } => {
                let text = String::from_utf8_lossy(&body);
                let protected =
                    result.is_ok() && hls::parse_media(&text, &url) == Err(Refusal::Protected);
                let result = if protected {
                    Err(Refusal::Protected)
                } else {
                    Ok(resolution)
                };
                self.media_update(tab, MediaEvent::Resolved { id: item, result });
            }
            Purpose::Playlist {
                job,
                track,
                url,
                body,
            } => {
                let parsed = result
                    .map_err(Refusal::Failed)
                    .and_then(|()| hls::parse_media(&String::from_utf8_lossy(&body), &url));
                match parsed {
                    Ok(plan) => {
                        if let Some(t) = self
                            .media
                            .runs
                            .get_mut(&job)
                            .and_then(|r| r.tracks.get_mut(track))
                        {
                            t.plan = Some(plan);
                        }
                        self.media_plan_ready(job);
                    }
                    Err(refusal) => self.job_event(job, JobEvent::Refused(refusal.message())),
                }
            }
            Purpose::Key { job, uri, body } => match result {
                Ok(()) if body.len() == 16 => {
                    if let Some(run) = self.media.runs.get_mut(&job) {
                        run.keys.insert(uri, body);
                    }
                    self.media_plan_ready(job);
                }
                Ok(()) => self.job_event(
                    job,
                    JobEvent::Refused("the stream's key is not an AES-128 key".into()),
                ),
                Err(e) => self.job_event(
                    job,
                    JobEvent::Refused(format!("cannot fetch the stream's key: {e}")),
                ),
            },
            Purpose::Segment { job, track, index } => match result {
                Ok(()) => {
                    self.media.write(WriteCmd::End { job, track, index });
                    self.job_event(job, JobEvent::SegmentDone { track, index });
                }
                Err(e) => self.job_event(job, JobEvent::SegmentFailed(e)),
            },
        }
    }

    /// A resolved HLS master: one variant's media playlist is read first (`Purpose::Probe`);
    /// without one to read (or when it cannot start) the master is listed as it is.
    fn media_probe(&mut self, tab: TabId, item: u32, resolution: Resolution) {
        let playlist = resolution.variants.iter().find_map(|v| match &v.source {
            wubble_core::media::Source::HlsPlaylist(url) => Some((url.clone(), v.headers.clone())),
            _ => None,
        });
        let Some((url, headers)) = playlist else {
            let result = Ok(resolution);
            self.media_update(tab, MediaEvent::Resolved { id: item, result });
            return;
        };
        let page = self.committed.get(&tab).cloned().unwrap_or_default();
        let spec = FetchSpec {
            url: url.clone(),
            referrer: page,
            headers,
            credentials: true,
            cached: true,
            ..FetchSpec::default()
        };
        let purpose = Purpose::Probe {
            tab,
            item,
            url,
            resolution: resolution.clone(),
            body: Vec::new(),
        };
        if self.media_fetch(tab, spec, purpose).is_err() {
            let result = Ok(resolution);
            self.media_update(tab, MediaEvent::Resolved { id: item, result });
        }
    }

    // ----- the pill and the menu

    /// The glyph was clicked: the downloader's entry for the visible tab.
    pub(crate) fn media_menu_requested(&mut self, x: f32, y: f32) {
        if let Some(tab) = self.model.visible_tab() {
            self.media_entry(tab, x, y);
        }
    }

    /// The downloader's one entry (the pill's glyph, the video strip's save button): the menu of
    /// the tab's streams at `x`, `y` (window px), or (ask-each-time off) the default quality of
    /// its first saveable stream at once.
    pub(super) fn media_entry(&mut self, tab: TabId, x: f32, y: f32) {
        let Some(window) = self.w.window.upgrade() else {
            return;
        };
        let Some(list) = self.media.tabs.get(&tab) else {
            return;
        };
        if !self.media.settings.ask_each_time {
            let first = list.items.iter().find_map(|i| match &i.found {
                Found::Ready(variants) => {
                    let choices = select::choices(variants);
                    select::choose(&choices, self.media.settings.quality).map(|c| (i.id, c))
                }
                _ => None,
            });
            if let Some((item, choice)) = first {
                self.save_media(tab, item, choice);
                return;
            }
        }
        let page_title = self
            .model
            .tabs
            .get(&tab)
            .map(crate::shell::tab_title)
            .unwrap_or_default();
        let page_title = name::page_title(&page_title).to_owned();
        let titles: Vec<String> = list
            .items
            .iter()
            .map(|i| item_title(i, &page_title))
            .collect();
        let rows: Vec<MediaRow> = list
            .items
            .iter()
            .zip(&titles)
            .filter(|(i, _)| !matches!(i.found, Found::Refused(Refusal::Failed(_))))
            .map(|(i, title)| MediaRow {
                item: i.id,
                title,
                found: &i.found,
            })
            .collect();
        let entries = crate::menu::media_entries(&rows);
        let scale = window.window().scale_factor();
        let at = ((x * scale) as i32, (y * scale) as i32);
        self.apply_menu(tab, crate::menu::MenuEvent::ShellRequested { at, entries });
        self.render_surfaces(&window);
    }

    /// The tab's streams, as the downloader lists them (none while it is off).
    pub(super) fn media_items(&self, tab: TabId) -> &[Detected] {
        match self.media.tabs.get(&tab) {
            Some(list) if self.media.settings.enabled => &list.items,
            _ => &[],
        }
    }

    /// The downloads entry of the latest download started from the tab's page.
    pub(super) fn media_latest(&self, tab: TabId) -> Option<i64> {
        let page = page_key(self.committed.get(&tab)?);
        self.media
            .runs
            .iter()
            .rev()
            .find(|(_, r)| r.tab == tab && page_key(&r.page) == page)
            .map(|(&id, _)| id)
    }

    /// `--smoke video`: downloads started from the tab.
    pub(super) fn media_runs_from(&self, tab: TabId) -> usize {
        self.media.runs.values().filter(|r| r.tab == tab).count()
    }

    /// `--smoke video`: the saved file of the tab's latest download, once it is done.
    pub(super) fn media_saved(&self, tab: TabId) -> Option<PathBuf> {
        let run = self.media.runs.get(&self.media_latest(tab)?)?;
        (run.job == Job::Done).then(|| run.output.clone())
    }

    /// A menu row (or the glyph) chose to save `choice` of the tab's `item`.
    pub(super) fn save_media(&mut self, tab: TabId, item: u32, choice: usize) {
        let Some(detected) = self.media.tabs.get(&tab).and_then(|l| l.get(item)).cloned() else {
            return;
        };
        let Found::Ready(variants) = &detected.found else {
            return;
        };
        let Some(chosen) = select::choices(variants).into_iter().nth(choice) else {
            return;
        };
        let page = self.committed.get(&tab).cloned().unwrap_or_default();
        let page_title = self
            .model
            .tabs
            .get(&tab)
            .map(crate::shell::tab_title)
            .unwrap_or_default();
        let title = item_title(&detected, name::page_title(&page_title));
        let note = chosen.height.map(|h| format!("{h}p")).unwrap_or_default();
        let tracks: Vec<Variant> = chosen
            .selection
            .parts()
            .into_iter()
            .map(|i| variants[i].clone())
            .collect();
        let pair = matches!(chosen.selection, Selection::Pair { .. });
        self.start_media(
            tab,
            page,
            title,
            note,
            tracks,
            chosen.container,
            if pair { None } else { chosen.size },
            &detected.url,
        );
    }

    /// Where saved media goes: the setting's folder, else the downloads folder; created.
    fn media_dir(&self) -> Option<PathBuf> {
        let dir = match self.media.settings.save_dir.as_str() {
            "" => self.downloads.dir()?.to_path_buf(),
            dir => PathBuf::from(dir),
        };
        std::fs::create_dir_all(&dir)
            .map_err(|e| eprintln!("wubble: {}: {e}", dir.display()))
            .ok()?;
        Some(dir)
    }

    /// A free path in the media folder for `file_name`.
    fn media_path(&self, file_name: &str) -> Option<PathBuf> {
        let dir = self.media_dir()?;
        download::unique_name(&dir, file_name, |p| {
            p.exists() || self.media.reserved(p) || self.downloads.reserved(p)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn start_media(
        &mut self,
        tab: TabId,
        page: String,
        title: String,
        note: String,
        variants: Vec<Variant>,
        container: Container,
        size: Option<u64>,
        url: &str,
    ) {
        let Some(output) = self.media_path(&name::media_file_name(&title, &note, container)) else {
            if let Some(window) = self.w.window.upgrade() {
                self.toast(&window, "Cannot save: no folder for videos");
            }
            return;
        };
        let tracks = variants
            .into_iter()
            .enumerate()
            .map(|(i, variant)| TrackRun {
                plan: variant.source.plan(variant.container),
                part: part_path(&output, i),
                variant,
            })
            .collect();
        let id = self.add_download_entry(url, &output, size);
        let run = Run {
            job: Job::Resolving { paused: false },
            tab,
            page,
            title,
            note,
            tracks,
            output,
            keys: HashMap::new(),
            fetched: BTreeMap::new(),
            size,
            saved: Vec::new(),
        };
        self.media.runs.insert(id, run);
        self.media_resolve_job(id);
    }

    /// Fetches what the job's plan still lacks: HLS media playlists, then AES-128 keys.
    fn media_resolve_job(&mut self, job: i64) {
        let Some(run) = self.media.runs.get(&job) else {
            return;
        };
        let (tab, page) = (run.tab, run.page.clone());
        let playlists: Vec<(usize, String, Headers)> = run
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.plan.is_none())
            .filter_map(|(i, t)| match &t.variant.source {
                wubble_core::media::Source::HlsPlaylist(url) => {
                    Some((i, url.clone(), t.variant.headers.clone()))
                }
                _ => None,
            })
            .collect();
        if playlists.is_empty() {
            self.media_plan_ready(job);
            return;
        }
        for (track, url, headers) in playlists {
            let spec = FetchSpec {
                url: url.clone(),
                referrer: page.clone(),
                headers,
                credentials: true,
                ..FetchSpec::default()
            };
            let purpose = Purpose::Playlist {
                job,
                track,
                url,
                body: Vec::new(),
            };
            if let Err(e) = self.media_fetch(tab, spec, purpose) {
                self.job_event(job, JobEvent::Refused(e));
                return;
            }
        }
    }

    /// The job's resolving fetches may all be in: fetch missing keys, or start fetching.
    fn media_plan_ready(&mut self, job: i64) {
        let resolving = self
            .media
            .fetches
            .values()
            .any(|p| p.job() == Some(job) && !matches!(p, Purpose::Segment { .. }));
        let Some(run) = self.media.runs.get(&job) else {
            return;
        };
        if resolving
            || !matches!(run.job, Job::Resolving { .. })
            || run.tracks.iter().any(|t| t.plan.is_none())
        {
            return;
        }
        let mut missing: Vec<String> = run
            .tracks
            .iter()
            .flat_map(|t| t.plan.iter().flat_map(|p| &p.segments))
            .filter_map(|s| s.key.as_ref().map(|k| k.uri.clone()))
            .filter(|uri| !run.keys.contains_key(uri))
            .collect();
        missing.sort();
        missing.dedup();
        if missing.is_empty() {
            let counts = run
                .tracks
                .iter()
                .map(|t| t.plan.as_ref().map_or(0, |p| p.segments.len()))
                .collect();
            let limit = self.media.settings.concurrency;
            self.job_event(job, JobEvent::Planned { counts, limit });
            return;
        }
        let (tab, page) = (run.tab, run.page.clone());
        for uri in missing {
            let spec = FetchSpec {
                url: uri.clone(),
                referrer: page.clone(),
                credentials: true,
                ..FetchSpec::default()
            };
            if let Err(e) = self.media_fetch(
                tab,
                spec,
                Purpose::Key {
                    job,
                    uri,
                    body: Vec::new(),
                },
            ) {
                self.job_event(job, JobEvent::Refused(e));
                return;
            }
        }
    }

    /// Runs `event` (and what its effects lead to) through the job's machine.
    fn job_event(&mut self, job: i64, event: JobEvent) {
        let mut queue = VecDeque::from([event]);
        while let Some(event) = queue.pop_front() {
            let Some(run) = self.media.runs.get_mut(&job) else {
                return;
            };
            let before = std::mem::discriminant(&run.job);
            let (next, effects) =
                job::transition(std::mem::replace(&mut run.job, Job::Cancelled), event);
            run.job = next;
            let changed = before != std::mem::discriminant(&run.job);
            let report = changed.then(|| job::list_event(&run.job)).flatten();
            for effect in effects {
                if let Some(e) = self.job_effect(job, effect) {
                    queue.push_back(e);
                }
            }
            if let Some(e) = report {
                self.apply_download(job, e);
            }
        }
    }

    /// Executes one job effect; a failure to start a fetch comes back as an event.
    fn job_effect(&mut self, job: i64, effect: JobEffect) -> Option<JobEvent> {
        let container = self
            .model
            .tabs
            .get(&tab)
            .map_or(ContainerId::DEFAULT, |t| t.container);
        let run = self.media.runs.get_mut(&job)?;
        match effect {
            container,
            JobEffect::Fetch { track, index } => {
                let t = run.tracks.get(track)?;
                let segment = t.plan.as_ref()?.segments.get(index)?.clone();
                let key = segment
                    .key
                    .as_ref()
                    .and_then(|k| Some((run.keys.get(&k.uri)?.clone(), k.iv)));
                run.fetched.remove(&(track, index));
                let spec = FetchSpec {
                    url: segment.url,
                    referrer: run.page.clone(),
                    range: segment.range.map(|r| (r.start, r.start + r.len.max(1) - 1)),
                    headers: t.variant.headers.clone(),
                    credentials: true,
                    ..FetchSpec::default()
                };
                let (tab, part) = (run.tab, t.part.clone());
                self.media.write(WriteCmd::Begin {
                    job,
                    track,
                    index,
                    path: part,
                    key,
                });
                self.media_fetch(tab, spec, Purpose::Segment { job, track, index })
                    .err()
                    .map(JobEvent::SegmentFailed)
            }
            JobEffect::StopFetches => {
                let ids: Vec<FetchId> = self
                    .media
                    .fetches
                    .iter()
                    .filter(|(_, p)| p.job() == Some(job))
        session: ContainerId,
                    .map(|(&id, _)| id)
                    .collect();
                for id in ids {
                    self.media.fetches.remove(&id);
                    self.media.fetcher.cancel(id);
                }
                None
            }
            JobEffect::Rewind { track, keep } => {
                run.fetched.retain(|&(t, i), _| t != track || i < keep);
                self.media.write(WriteCmd::Rewind { job, track, keep });
                None
            }
            JobEffect::CloseTrack(track) => {
                self.media.write(WriteCmd::Close { job, track });
                None
            }
            JobEffect::Join => {
                let run = self.media.runs.get(&job)?;
                let container = run
                    .tracks
                    .first()
                    .map(|t| t.plan.as_ref().map_or(t.variant.container, |p| p.container))?;
                let (title, note) = (run.title.clone(), run.note.clone());
                let parts: Vec<PathBuf> = run.tracks.iter().map(|t| t.part.clone()).collect();
                let containers: Vec<Container> = run
                    .tracks
            container: session,
                    .iter()
                    .map(|t| t.plan.as_ref().map_or(t.variant.container, |p| p.container))
                    .collect();
                let current = run.output.clone();
                // The tracks named the container: the file takes its extension.
                let output =
                    if current.extension().and_then(|e| e.to_str()) == Some(container.ext()) {
                        current
                    } else {
                        self.media_path(&name::media_file_name(&title, &note, container))
                            .unwrap_or(current)
                    };
                let fallback = if parts.len() == 2 {
                    [("video", containers[0]), ("audio", containers[1])]
                        .iter()
                        .filter_map(|(what, c)| {
                            self.media_path(&name::media_file_name(&title, what, *c))
                        })
                        .collect()
                } else {
                    vec![]
                };
                if let Some(run) = self.media.runs.get_mut(&job) {
                    run.output = output.clone();
                }
                self.media.write(WriteCmd::Join(Join {
                    job,
                    parts,
                    output,
                    container,
                    fallback,
                }));
                None
            }
            JobEffect::Discard => {
                let parts = run.tracks.iter().map(|t| t.part.clone());
                let paths = parts.chain(std::mem::take(&mut run.saved)).collect();
                self.media.write(WriteCmd::Discard { job, paths });
                None
            }
        }
    }

    fn media_joined(&mut self, job: i64, result: Result<Vec<PathBuf>, String>) {
        let Some(run) = self.media.runs.get_mut(&job) else {
            return;
        };
        let kept = run.job == Job::Joining { cancelled: false };
        if let Ok(paths) = &result {
            run.saved.clone_from(paths);
            if kept && let Some(first) = paths.first() {
                self.set_download_path(job, first);
            }
            if kept
                && paths.len() == 2
                && let Some(window) = self.w.window.upgrade()
            {
                self.toast(
                    &window,
                    "ffmpeg not found: video and audio saved as two files",
                );
            }
        }
        self.job_event(job, JobEvent::Joined(result.map(|_| ())));
        self.media_progress(job);
        // The entry went while the join ran (removed, or replaced by a retry): so does the run.
        if self.download_url(job).is_none() {
            self.media.runs.remove(&job);
        }
    }

    /// The entry of `id` goes (removed, or replaced by a retry): so does its run, unless its
    /// join still runs; then the run, and the paths it reserves, stay until it ends.
    fn media_release(&mut self, id: i64) {
        if !matches!(
            self.media.runs.get(&id).map(|r| &r.job),
            Some(Job::Joining { .. })
        ) {
            self.media.runs.remove(&id);
        }
    }

    /// The list's buttons on a media entry (`DownloadEffect`s of its machine).
    pub(super) fn media_list_effect(&mut self, id: i64, effect: DownloadEffect) {
        match effect {
            DownloadEffect::PauseEngine => self.job_event(id, JobEvent::Pause),
            DownloadEffect::ResumeEngine => self.job_event(id, JobEvent::Resume),
            DownloadEffect::CancelEngine => self.job_event(id, JobEvent::Cancel),
            DownloadEffect::StartAgain => self.media_restart(id),
            DownloadEffect::Forget => {
                if self.media.runs.get(&id).is_some_and(|r| r.job.is_live()) {
                    self.job_effect(id, JobEffect::Discard);
                }
                self.media_release(id);
            }
            DownloadEffect::Save => {}
        }
    }

    /// Retry of a failed or cancelled media download: the same tracks as a new download, which
    /// replaces the entry.
    fn media_restart(&mut self, id: i64) {
        let Some(run) = self.media.runs.get(&id) else {
            return;
        };
        let file = run
            .tracks
            .first()
            .map_or(Container::Other, |t| t.variant.container);
        let variants: Vec<Variant> = run.tracks.iter().map(|t| t.variant.clone()).collect();
        let (tab, container, page, title, note, size) = (
            run.tab,
            run.page.clone(),
            run.title.clone(),
            run.note.clone(),
            run.size,
        );
        self.media_release(id);
        let url = self.download_url(id).unwrap_or_default();
        self.forget_download(id);
        self.start_media(tab, container, page, title, note, variants, file, size, &url);
    }
}

/// A stream's title for its menu line and file name: its own, else the page's.
fn item_title(item: &Detected, page_title: &str) -> String {
    match &item.title {
        Some(t) if !t.trim().is_empty() => t.clone(),
        _ if !page_title.is_empty() => page_title.to_owned(),
        _ => name::url_title(&item.url),
    }
}

/// `<output>.part<n>`, next to the output.
fn part_path(output: &Path, track: usize) -> PathBuf {
    let mut name = output.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".part{track}"));
    output.with_file_name(name)
}

/// Whether a retried entry of an earlier run is a stream (a manifest or a site's page) that
/// only the page can offer again, rather than a file the engine can download.
pub(super) fn is_stream_entry(url: &str) -> bool {
    matches!(
        wubble_core::media::path_ext(url).as_deref(),
        Some("m3u8" | "mpd")
    ) || youtube::video_id(url).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_sit_next_to_the_output() {
        assert_eq!(
            part_path(Path::new("/d/T (720p).mp4"), 1),
            Path::new("/d/T (720p).mp4.part1")
        );
    }

    #[test]
    fn streams_of_an_earlier_run_are_not_refetched_as_files() {
        assert!(is_stream_entry("https://a.test/x/master.m3u8?t=1"));
        assert!(is_stream_entry(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        ));
        assert!(!is_stream_entry("https://a.test/v.mp4"));
    }

    #[test]
    fn titles_prefer_the_stream_then_the_page() {
        let item = |title: Option<&str>| Detected {
            id: 0,
            url: "https://a.test/v/clip.mp4".into(),
            kind: Kind::File,
            title: title.map(str::to_owned),
            found: Found::Resolving,
        };
        assert_eq!(item_title(&item(Some("Own")), "Page"), "Own");
        assert_eq!(item_title(&item(None), "Page"), "Page");
        assert_eq!(item_title(&item(Some(" ")), ""), "clip");
    }
}
            run.container,
