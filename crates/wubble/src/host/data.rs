//! `--smoke data` (`data_smoke.rs`): the built-in pages and the data behind them, one script of
//! steps. A step has an `act` (run once, through the host's own paths or the window's own
//! callbacks, as a click sends them) and a `want` (polled after each run of CEF work until it
//! passes, fails or the run budget ends). Everything a step observes is what the person would
//! see (the pages' models, a tab's title, a file on disk) or what the store holds.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use slint::Model;
use wubble_core::action::Action;
use wubble_core::archive::ArchiveAfter;
use wubble_core::download::DownloadState;
use wubble_core::model::Command;
use wubble_core::pages::InternalPage;
use wubble_core::tab::TabId;
use wubble_ui::{AppWindow, DownloadAction, TaskAction};

use super::{Host, now_ms};
use crate::data_smoke::{self, Ending, Server, UNI_TITLE};
use crate::persist::VisitsPage;

/// A store answer, filled on the store thread.
type Probe<T> = Arc<Mutex<Option<T>>>;

fn probe<T>() -> Probe<T> {
    Arc::new(Mutex::new(None))
}

fn filled<T>(p: &Probe<T>) -> Option<T> {
    p.lock().ok()?.take()
}

fn fill<T: Send + 'static>(p: &Probe<T>) -> impl FnOnce(T) + Send + 'static {
    let p = Arc::clone(p);
    move |v| {
        if let Ok(mut slot) = p.lock() {
            *slot = Some(v);
        }
    }
}

/// What a step's `want` says.
pub(super) enum Poll {
    Pending,
    Pass,
    Fail(String),
}

use Poll::{Fail, Pass, Pending};

fn when(cond: bool) -> Poll {
    if cond { Pass } else { Pending }
}

fn expect(cond: bool, what: impl FnOnce() -> String) -> Poll {
    if cond { Pass } else { Fail(what()) }
}

/// State the steps share.
pub(crate) struct DataSmoke {
    server: Arc<Server>,
    ending: Ending,
    step: usize,
    runs: u32,
    entered: bool,
    /// The tab the web pages load in.
    web: Option<TabId>,
    /// A tab of another site, for the task manager to end.
    victim: Option<TabId>,
    visits: Probe<VisitsPage>,
    archive: Probe<VisitsPage>,
    bookmarks: Probe<Option<crate::persist::BookmarksPage>>,
    /// The id of the download entry a step follows.
    download: Option<i64>,
    /// Where the stand-in for the desktop's opener logs what it was asked to open.
    opener_log: PathBuf,
    /// The archive row's key a step restores or deletes.
    key: i32,
    /// Counts a step compares with: rows before an action.
    before: usize,
    /// Notes for the report: what a step observed that is not a failure.
    pub(crate) notes: Vec<String>,
}

impl DataSmoke {
    pub(crate) fn new(server: Arc<Server>, ending: Ending) -> Self {
        Self {
            server,
            ending,
            step: 0,
            runs: 0,
            entered: false,
            web: None,
            victim: None,
            visits: probe(),
            archive: probe(),
            bookmarks: probe(),
            download: None,
            opener_log: PathBuf::new(),
            key: -1,
            before: 0,
            notes: vec![],
        }
    }
}

struct Step {
    name: &'static str,
    act: fn(&mut Host, &mut DataSmoke),
    want: fn(&mut Host, &mut DataSmoke) -> Poll,
}

macro_rules! step {
    ($name:expr, $act:expr, $want:expr) => {
        Step {
            name: $name,
            act: $act,
            want: $want,
        }
    };
}

fn nothing(_: &mut Host, _: &mut DataSmoke) {}

/// Presses what the window's own callback does, later and outside the host it calls into.
fn ui(host: &Host, f: impl FnOnce(&AppWindow) + Send + 'static) {
    let _ = host.w.window.upgrade_in_event_loop(move |w| f(&w));
}

/// The scenario, in order. Each step is named in the failure line.
const STEPS: &[Step] = &[
    step!("ready", Host::data_start, |h, _| when(
        h.frame_rate.is_some()
    )),
    // ---- history: what a visit records
    step!("visit /page", |h, c| h.data_nav(c, "/page"), |h, c| h
        .data_title(c, "Page One")),
    step!("visit /uni", |h, c| h.data_nav(c, "/uni"), |h, c| h
        .data_title(c, UNI_TITLE)),
    step!("visit /other", |h, c| h.data_nav(c, "/other"), |h, c| h
        .data_title(c, "Page Two")),
    step!(
        "the store holds the three visits",
        |h, c| h.store.visits("", None, 50, fill(&c.visits)),
        Host::data_visits_true
    ),
    // ---- bookmarks: the sheet, a folder, a duplicate, rename, move, delete
    step!(
        "bookmark /other with Mod+D",
        |h, _| h.bookmark_page(),
        |h, _| when(h.bookmark_sheet_open())
    ),
    step!(
        "answer the sheet: a name",
        |h, _| ui(h, |w| w.invoke_bookmark_sheet_answered(
            true,
            false,
            "Zwei ÄÖ".into(),
            -1
        )),
        |h, _| when(h.bookmark_names().contains(&"Zwei ÄÖ".to_owned()))
    ),
    step!(
        "new folder",
        |h, _| ui(h, |w| w.invoke_bookmark_folder_requested()),
        |h, _| when(h.bookmark_sheet_open())
    ),
    step!(
        "answer the sheet: the folder's name",
        |h, _| ui(h, |w| w.invoke_bookmark_sheet_answered(
            true,
            false,
            "Ordner".into(),
            -1
        )),
        |h, _| when(h.bookmark_names().contains(&"Ordner".to_owned()))
    ),
    step!(
        "bookmark /other again: it is the same bookmark",
        |h, _| h.bookmark_page(),
        |h, _| when(h.bookmark_sheet_open())
    ),
    step!(
        "answer with a new name: renamed, not a duplicate",
        |h, _| ui(h, |w| w.invoke_bookmark_sheet_answered(
            true,
            false,
            "Zwei".into(),
            -1
        )),
        |h, _| {
            let names = h.bookmark_names();
            if names.contains(&"Zwei".to_owned()) {
                expect(
                    names.len() == 2 && !names.contains(&"Zwei ÄÖ".to_owned()),
                    || format!("a duplicate or a stale name: {names:?}"),
                )
            } else {
                Pending
            }
        }
    ),
    // ---- every page from Settings' Pages group
    step!(
        "Settings' links open their pages",
        |_, c| {
            c.before = 0;
            c.key = 0;
        },
        Host::data_links
    ),
    // ---- history page: open, rows, search
    step!(
        "open History by its command",
        |h, _| h.perform(Action::OpenHistory),
        |h, _| h.data_history_rows(4)
    ),
    step!(
        "search: page one",
        |h, _| h.data_query("page one"),
        |h, _| h.data_history_visits(1)
    ),
    step!("search: PAGE", |h, _| h.data_query("PAGE"), |h, _| h
        .data_history_visits(2)),
    step!(
        "search: привет",
        |h, _| h.data_query("привет"),
        |h, _| h.data_history_visits(1)
    ),
    step!("search: zzz", |h, _| h.data_query("zzz"), |h, _| h
        .data_history_visits(0)),
    step!(
        "search: ПРИВЕТ",
        |h, _| h.data_query("ПРИВЕТ"),
        |h, _| h.data_history_visits(1)
    ),
    step!("search: %", |h, _| h.data_query("%"), |h, _| h
        .data_history_visits(0)),
    step!("search: ärger", |h, _| h.data_query("ärger"), |h, _| h
        .data_history_visits(1)),
    step!("search: '", |h, _| h.data_query("'"), |h, _| h
        .data_history_visits(0)),
    step!("search: /uni", |h, _| h.data_query("/uni"), |h, _| h
        .data_history_visits(1)),
    step!("search: 日本", |h, _| h.data_query("日本"), |h, _| h
        .data_history_visits(0)),
    step!(
        "search: the host",
        |h, _| h.data_query("127.0.0.1:"),
        |h, _| h.data_history_visits(3)
    ),
    step!("search: zzz again", |h, _| h.data_query("zzz"), |h, _| h
        .data_history_visits(0)),
    step!("search cleared", |h, _| h.data_query("  "), |h, _| h
        .data_history_visits(3)),
    // ---- one row deleted
    step!(
        "delete /uni from the page",
        |h, c| {
            let url = h.server_url(c, "/uni");
            ui(h, move |w| w.invoke_history_deleted(url.as_str().into()));
        },
        |h, _| h.data_history_visits(2)
    ),
    step!(
        "that visit is gone from the store",
        |h, c| h.store.visits("", None, 50, fill(&c.visits)),
        |_, c| match filled(&c.visits) {
            None => Pending,
            Some(p) => expect(
                p.rows.len() == 2 && p.rows.iter().all(|r| !r.url.ends_with("/uni")),
                || format!("rows after a delete: {:?}", p.rows),
            ),
        }
    ),
    // ---- cookies and cache: clearing them is observable
    step!("set a cookie", |h, c| h.data_nav(c, "/set"), |h, c| h
        .data_title(c, "cookie set")),
    step!(
        "the cookie reaches the server",
        |h, c| h.data_nav(c, "/echo"),
        |h, c| h.data_title(c, "cookie=sid=abc123")
    ),
    step!(
        "load a cacheable page twice",
        |h, c| h.data_nav(c, "/count"),
        |h, c| h.data_title(c, "n=1")
    ),
    step!(
        "elsewhere and back: the cache answers",
        |h, c| h.data_nav(c, "/page"),
        |h, c| h.data_title(c, "Page One")
    ),
    step!(
        "the cached page is not fetched again",
        |h, c| h.data_nav(c, "/count"),
        |h, c| match h.data_web_title(c) {
            Some(t) if t == "n=1" => expect(c.server.counted() == 1, || {
                format!("served {} times, not once", c.server.counted())
            }),
            Some(t) if t == "Page One" => Pending,
            Some(t) => Fail(format!("the cacheable page shows {t:?}")),
            None => Pending,
        }
    ),
    step!(
        "bookmarks and the open tab before clearing: note the counts",
        |h, c| {
            c.before = h.bookmark_names().len();
            h.store.bookmarks(None, fill(&c.bookmarks));
        },
        |_, c| when(filled(&c.bookmarks).is_some())
    ),
    step!(
        "clear cookies and cache (the menu's row)",
        |h, c| {
            let containers = h.model.containers.iter().map(|k| k.id).collect();
            if let Some(tab) = c.web {
                h.clear_data(tab, crate::menu::ClearData::Containers(containers));
            }
        },
        nothing_to_wait
    ),
    step!(
        "the cookie no longer reaches the server",
        |h, c| h.data_nav(c, "/echo"),
        |h, c| match h.data_web_title(c) {
            Some(t) if t == "cookie=none" => Pass,
            Some(t) if t == "cookie=sid=abc123" => {
                if c.runs > 600 {
                    Fail("cookies survived the clear: the server still got sid".into())
                } else {
                    Pending
                }
            }
            _ => Pending,
        }
    ),
    step!(
        "the cached page is fetched again",
        |h, c| h.data_nav(c, "/count"),
        |h, c| match h.data_web_title(c) {
            Some(t) if t == "n=2" => Pass,
            Some(t) if t == "n=1" => {
                if c.runs > 600 {
                    Fail("the cache survived the clear: /count was answered from it".into())
                } else {
                    Pending
                }
            }
            _ => Pending,
        }
    ),
    // ---- clear history by range: visits go, the rest stays
    step!(
        "History again",
        |h, _| h.perform(Action::OpenHistory),
        |h, _| when(h.shown_page() == Some(InternalPage::History))
    ),
    step!(
        "clear browsing data: all time",
        |h, _| ui(h, |w| w.invoke_history_cleared(2)),
        |h, _| h.data_history_rows(0)
    ),
    step!(
        "the store has no visits",
        |h, c| h.store.visits("", None, 50, fill(&c.visits)),
        |_, c| match filled(&c.visits) {
            None => Pending,
            Some(p) => expect(p.rows.is_empty(), || format!("visits left: {:?}", p.rows)),
        }
    ),
    step!(
        "bookmarks survived the clear",
        |h, c| h.store.bookmarks(None, fill(&c.bookmarks)),
        |h, c| match filled(&c.bookmarks) {
            None => Pending,
            Some(p) => {
                let n = p.map_or(0, |p| p.rows.len());
                expect(n == c.before && n == 2, || {
                    format!("bookmarks after clearing history: {n} (was {})", c.before)
                })
                .and(h.data_web_alive(c))
            }
        }
    ),
    // ---- downloads
    step!(
        "downloads folder: gone",
        Host::data_remove_downloads_dir,
        nothing_to_wait
    ),
    step!(
        "a small download with the folder missing",
        |h, c| h.data_nav(c, "/small"),
        |h, c| h.data_download_state(c, |s| matches!(s, DownloadState::Complete), "complete")
    ),
    step!("the file is where the page says", nothing, |h, c| h
        .data_file_exists(c, 100)),
    step!(
        "a held download shows progress",
        |h, c| {
            c.server.release(false);
            h.data_nav(c, "/file/held.bin");
        },
        |h, c| h.data_progress(c)
    ),
    step!(
        "pause",
        |h, c| h.data_action(c, DownloadAction::Pause),
        |h, c| h.data_download_state(c, |s| matches!(s, DownloadState::Paused), "paused")
    ),
    step!(
        "resume",
        |h, c| h.data_action(c, DownloadAction::Resume),
        |h, c| h.data_download_state(c, |s| matches!(s, DownloadState::Running), "running")
    ),
    step!(
        "release: it completes with all its bytes",
        |_, c| c.server.release(true),
        |h, c| {
            let ok = h.data_download_state(c, |s| matches!(s, DownloadState::Complete), "complete");
            match ok {
                Pass => h.data_file_exists(c, data_smoke::FILE_BYTES as u64),
                other => other,
            }
        }
    ),
    step!(
        "a second held download, cancelled",
        |h, c| {
            c.server.release(false);
            h.data_nav(c, "/file/cancel.bin");
        },
        |h, c| h.data_progress(c)
    ),
    step!(
        "cancel",
        |h, c| h.data_action(c, DownloadAction::Cancel),
        |h, c| h.data_download_state(c, |s| matches!(s, DownloadState::Cancelled), "cancelled")
    ),
    step!(
        "retry starts it again and replaces the entry",
        |h, c| {
            c.before = h.downloads.started();
            h.data_action(c, DownloadAction::Retry);
        },
        |h, c| h.data_retried(c)
    ),
    step!(
        "release: the retry completes",
        |_, c| c.server.release(true),
        |h, c| h.data_download_state(c, |s| matches!(s, DownloadState::Complete), "complete")
    ),
    step!(
        "a download cut off after 8 KiB",
        |h, c| h.data_nav(c, "/fail"),
        |h, c| h.data_download_ends_badly(c)
    ),
    step!(
        "remove the finished entry from the list",
        Host::data_remove_first,
        |h, c| { h.data_removed(c) }
    ),
    step!(
        "downloads folder: read-only",
        Host::data_lock_downloads_dir,
        nothing_to_wait
    ),
    step!(
        "a download into a read-only folder says so",
        |h, c| {
            c.before = h.downloads.started();
            h.data_nav(c, "/small");
        },
        |h, c| h.data_readonly_outcome(c)
    ),
    step!(
        "downloads folder: writable again",
        Host::data_unlock_downloads_dir,
        nothing_to_wait
    ),
    step!(
        "open and show-in-folder go to the opener, once each",
        Host::data_open_and_reveal,
        |h, c| h.data_opener_asked(c)
    ),
    step!(
        "a missing file is not opened",
        Host::data_open_missing,
        |h, c| h.data_opener_not_asked(c)
    ),
    step!(
        "open the Downloads page by its address",
        |h, c| h.data_nav(c, "wubble://downloads"),
        |h, _| when(h.shown_page() == Some(InternalPage::Downloads))
    ),
    step!(
        "Clear finished leaves only what runs",
        |h, _| ui(h, |w| w.invoke_downloads_cleared()),
        |h, _| h.data_downloads_all_active()
    ),
    // ---- the archive
    step!(
        "the archive setting, 12 hours, through its row",
        Host::data_archive_setting,
        |h, _| { when(h.archive_after == ArchiveAfter::Hours12) }
    ),
    step!(
        "a tab to put away",
        |h, c| {
            let url = h.server_url(c, "/page");
            h.dispatch(Command::NewTab {
                url,
                container: None,
                opener: None,
            });
        },
        |h, _| when(h.data_tab_titled("Page One").is_some())
    ),
    step!(
        "another tab on top of it",
        |h, c| {
            let url = h.server_url(c, "/other");
            h.dispatch(Command::NewTab {
                url,
                container: None,
                opener: None,
            });
            c.web = h.shown();
        },
        |h, c| h.data_title(c, "Page Two")
    ),
    step!(
        "13 hours unvisited: filed in the archive and closed",
        Host::data_age_and_archive,
        |h, c| h.data_archived(c)
    ),
    step!(
        "open the Archive by its command",
        |h, _| h.perform(Action::OpenArchive),
        |h, _| h.data_archive_rows(2)
    ),
    step!(
        "restore it from the page",
        |h, c| {
            c.key = h.data_archive_key().unwrap_or(-1);
            let key = c.key;
            ui(h, move |w| w.invoke_archive_restored(key));
        },
        |h, c| h.data_restored(c)
    ),
    step!(
        "the Archive page is empty again",
        |h, _| h.perform(Action::OpenArchive),
        |h, _| h.data_archive_rows(0)
    ),
    step!(
        "put it away again, with the page open",
        Host::data_age_and_archive_restored,
        |h, c| h.data_archived_again(c)
    ),
    step!(
        "delete the archived tab",
        |h, c| {
            c.key = h.data_archive_key().unwrap_or(-1);
            let key = c.key;
            ui(h, move |w| w.invoke_archive_deleted(key));
        },
        |h, _| h.data_archive_rows(0)
    ),
    step!(
        "the archive store is empty",
        |h, c| h.store.archived("", None, 50, fill(&c.archive)),
        |_, c| match filled(&c.archive) {
            None => Pending,
            Some(p) => expect(p.rows.is_empty(), || format!(
                "archive rows left: {:?}",
                p.rows
            )),
        }
    ),
    // ---- the task manager
    step!(
        "a tab on another site, for the task manager",
        |h, c| {
            let url = format!("http://t1.localhost:{}/page", c.server.port);
            h.dispatch(Command::NewTab {
                url,
                container: None,
                opener: None,
            });
            c.victim = h.shown();
        },
        |h, c| h.data_victim_ready(c)
    ),
    step!(
        "open the Task manager",
        |h, _| h.perform(Action::OpenTasks),
        |h, c| h.data_tasks_listed(c)
    ),
    step!(
        "end the newest renderer from its process row",
        Host::data_end_victim,
        |h, c| h.data_victim_ended(c)
    ),
    // ---- the agent activity page
    step!(
        "an agent's call is logged",
        |h, _| h.data_log_call("navigate"),
        nothing_to_wait
    ),
    step!(
        "open Agent activity",
        |h, _| h.perform(Action::OpenAgentLog),
        |h, _| h.data_agent_rows(1)
    ),
    step!(
        "a second call shows while the page is open",
        |h, _| h.data_log_call("read_page"),
        |h, _| h.data_agent_rows(2)
    ),
    // ---- the end: what the pages show is written for the parent to compare with the store
    step!(
        "History at the end",
        |h, _| h.perform(Action::OpenHistory),
        |h, c| when(
            c.runs > 20
                && h.shown_page() == Some(InternalPage::History)
                && h.w
                    .window
                    .upgrade()
                    .is_some_and(|w| w.get_history_lines().row_count() > 0)
        )
    ),
    step!("write the summary", Host::data_summary, nothing_to_wait),
    step!(
        "the store answered everything sent so far",
        |h, c| h.store.visits("", None, 1, fill(&c.visits)),
        |_, c| when(filled(&c.visits).is_some())
    ),
];

fn nothing_to_wait(_: &mut Host, _: &mut DataSmoke) -> Poll {
    Pass
}

impl Poll {
    fn and(self, next: Poll) -> Poll {
        match self {
            Pass => next,
            other => other,
        }
    }
}

impl Host {
    /// One run of CEF work: the current step's act (once), then its want.
    pub(super) fn data_smoke_step(&mut self) {
        let Some(mut c) = self.checks.data_smoke.take() else {
            return;
        };
        let Some(step) = STEPS.get(c.step) else {
            self.checks.data_smoke = Some(c);
            return;
        };
        if !c.entered {
            c.entered = true;
            (step.act)(self, &mut c);
        }
        c.runs += 1;
        let poll = (step.want)(self, &mut c);
        let name = step.name;
        match poll {
            Pass => {
                println!("smoke: data: step {} ok: {name}", c.step);
                c.step += 1;
                c.runs = 0;
                c.entered = false;
                if c.step == STEPS.len() {
                    self.data_end(c.ending, &c.notes);
                    return;
                }
            }
            Fail(why) => {
                for note in &c.notes {
                    println!("smoke: data: note: {note}");
                }
                self.fail(format!("step {} ({name}): {why}", c.step));
                return;
            }
            Pending if c.runs >= data_smoke::MAX_STEP_RUNS => {
                for note in &c.notes {
                    println!("smoke: data: note: {note}");
                }
                self.fail(format!(
                    "step {} ({name}): not seen after {} runs of CEF work",
                    c.step,
                    data_smoke::MAX_STEP_RUNS
                ));
                return;
            }
            Pending => {
                if c.runs % 1000 == 0 {
                    let first =
                        self.downloads.entries().first().map(|e| {
                            format!("{} {:?} {}/{:?}", e.name, e.state, e.received, e.total)
                        });
                    eprintln!(
                        "smoke: data: step {} ({name}) waits at run {}: shown {:?}, first download {first:?}, /fail asked {}",
                        c.step,
                        c.runs,
                        self.shown_page(),
                        c.server.fails()
                    );
                }
            }
        }
        self.checks.data_smoke = Some(c);
    }

    fn data_end(&mut self, ending: Ending, notes: &[String]) {
        for note in notes {
            println!("smoke: data: note: {note}");
        }
        match ending {
            Ending::Quit => {
                self.data_passed = true;
                crate::quit();
            }
            Ending::Kill => {
                // the store answered the last read: everything before it is on disk
                println!("smoke: data: killing the process");
                let pid = std::process::id().to_string();
                let _ = std::process::Command::new("kill")
                    .args(["-9", &pid])
                    .status();
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
        }
    }

    fn server_url(&self, c: &DataSmoke, path: &str) -> String {
        c.server.url(path)
    }

    fn data_profile(&self) -> PathBuf {
        self.downloads
            .dir()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }

    fn data_start(&mut self, c: &mut DataSmoke) {
        c.web = self.shown();
        c.opener_log = self.data_profile().join("opener.log");
    }

    fn data_nav(&mut self, c: &mut DataSmoke, path: &str) {
        let url = if path.starts_with('/') {
            c.server.url(path)
        } else {
            path.to_owned()
        };
        if let Some(tab) = c.web {
            self.dispatch(Command::Navigate { tab, url });
        }
    }

    fn data_web_title(&self, c: &DataSmoke) -> Option<String> {
        self.model.tabs.get(&c.web?).map(|t| t.title.clone())
    }

    fn data_title(&self, c: &DataSmoke, want: &str) -> Poll {
        when(self.data_web_title(c).as_deref() == Some(want))
    }

    fn data_web_alive(&self, c: &DataSmoke) -> Poll {
        let alive = c.web.is_some_and(|t| self.model.tabs.contains_key(&t));
        expect(alive, || "the open tab went with the clear".into())
    }

    fn data_visits_true(&mut self, c: &mut DataSmoke) -> Poll {
        let Some(p) = filled(&c.visits) else {
            return Pending;
        };
        let urls: Vec<&str> = p.rows.iter().map(|r| r.url.as_str()).collect();
        let want = ["/other", "/uni", "/page"];
        if urls.len() != 3 || !urls.iter().zip(want).all(|(u, w)| u.ends_with(w)) {
            return Fail(format!("visits {urls:?}, newest first expected {want:?}"));
        }
        let titles: Vec<&str> = p.rows.iter().map(|r| r.title.as_str()).collect();
        if titles != ["Page Two", UNI_TITLE, "Page One"] {
            return Fail(format!("titles {titles:?}"));
        }
        let now = now_ms();
        match p
            .rows
            .iter()
            .find(|r| !(0..=10 * 60_000).contains(&(now - r.visit_ms)))
        {
            Some(r) => Fail(format!(
                "visit time {} for {} is not now ({now})",
                r.visit_ms, r.url
            )),
            None => Pass,
        }
    }

    // ---- history page

    fn data_query(&mut self, query: &str) {
        let q = query.to_owned();
        if let Some(w) = self.w.window.upgrade() {
            // the search field's own callback
            let _ = self
                .w
                .window
                .upgrade_in_event_loop(move |w| w.invoke_history_query_changed(q.as_str().into()));
            drop(w);
        }
    }

    fn data_history_rows(&self, rows: usize) -> Poll {
        let Some(w) = self.w.window.upgrade() else {
            return Pending;
        };
        when(
            self.shown_page() == Some(InternalPage::History)
                && w.get_history_lines().row_count() == rows,
        )
    }

    /// The page shows this many visits (its day header apart).
    fn data_history_visits(&self, visits: usize) -> Poll {
        let rows = if visits == 0 { 0 } else { visits + 1 };
        self.data_history_rows(rows)
    }

    // ---- downloads

    fn data_remove_downloads_dir(&mut self, _: &mut DataSmoke) {
        if let Some(dir) = self.downloads.dir() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    fn data_lock_downloads_dir(&mut self, _: &mut DataSmoke) {
        use std::os::unix::fs::PermissionsExt;
        if let Some(dir) = self.downloads.dir() {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555));
        }
    }

    fn data_unlock_downloads_dir(&mut self, _: &mut DataSmoke) {
        use std::os::unix::fs::PermissionsExt;
        if let Some(dir) = self.downloads.dir() {
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
        }
    }

    fn data_first(&self) -> Option<&wubble_core::download::DownloadEntry> {
        self.downloads.entries().first()
    }

    fn data_download_state(
        &mut self,
        c: &mut DataSmoke,
        is: fn(&DownloadState) -> bool,
        word: &str,
    ) -> Poll {
        let Some(e) = self.data_first() else {
            return Pending;
        };
        c.download = Some(e.id);
        if is(&e.state) {
            Pass
        } else if matches!(e.state, DownloadState::Failed(_)) && word != "failed" {
            Fail(format!(
                "the download failed instead of {word}: {:?}",
                e.state
            ))
        } else {
            Pending
        }
    }

    fn data_file_exists(&self, _: &DataSmoke, bytes: u64) -> Poll {
        let Some(e) = self.data_first() else {
            return Pending;
        };
        match std::fs::metadata(&e.path) {
            Ok(m) if m.len() == bytes => Pass,
            Ok(m) => Fail(format!(
                "{} has {} bytes, not {bytes}",
                e.path.display(),
                m.len()
            )),
            Err(err) => Fail(format!("{}: {err}", e.path.display())),
        }
    }

    fn data_progress(&mut self, c: &mut DataSmoke) -> Poll {
        let Some(e) = self.data_first() else {
            return Pending;
        };
        c.download = Some(e.id);
        when(e.state == DownloadState::Running && e.received > 0)
    }

    fn data_action(&mut self, c: &mut DataSmoke, action: DownloadAction) {
        if let Some(id) = c.download.or_else(|| self.data_first().map(|e| e.id)) {
            let id = i32::try_from(id).unwrap_or(-1);
            ui(self, move |w| w.invoke_download_action(id, action));
        }
    }

    fn data_retried(&mut self, c: &mut DataSmoke) -> Poll {
        let Some(e) = self.data_first() else {
            return Pending;
        };
        if Some(e.id) == c.download {
            return Pending;
        }
        if e.state != DownloadState::Running {
            return Pending;
        }
        let (id, n) = (e.id, self.downloads.started());
        c.download = Some(id);
        expect(n == c.before, || {
            format!("a retry left {n} entries where there were {}", c.before)
        })
    }

    fn data_download_ends_badly(&mut self, c: &mut DataSmoke) -> Poll {
        let Some(e) = self.data_first() else {
            return Pending;
        };
        if e.name != "fail.bin" {
            return Pending;
        }
        c.download = Some(e.id);
        match &e.state {
            // CEF resumes a cut-off download itself, with backoff, before it gives up
            DownloadState::Running if c.runs > 2_500 => {
                c.notes.push(
                    "a download the server cut off still shows Running (CEF resumes it \
                     itself; the second attempt was cut again): cancelled by the step"
                        .into(),
                );
                let id = i32::try_from(e.id).unwrap_or(-1);
                ui(self, move |w| {
                    w.invoke_download_action(id, DownloadAction::Cancel)
                });
                Pass
            }
            DownloadState::Running => Pending,
            DownloadState::Interrupted(why) | DownloadState::Failed(why) => {
                c.notes
                    .push(format!("a cut-off download reads {:?} ({why})", e.state));
                Pass
            }
            other => Fail(format!("a cut-off download reads {other:?}")),
        }
    }

    fn data_remove_first(&mut self, c: &mut DataSmoke) {
        if let Some(id) = self.data_first().map(|e| e.id) {
            c.download = Some(id);
            c.before = self.downloads.started();
            let id = i32::try_from(id).unwrap_or(-1);
            ui(self, move |w| {
                w.invoke_download_action(id, DownloadAction::Remove)
            });
        }
    }

    fn data_removed(&self, c: &DataSmoke) -> Poll {
        let gone = !self
            .downloads
            .entries()
            .iter()
            .any(|e| Some(e.id) == c.download);
        when(gone && self.downloads.started() + 1 == c.before)
    }

    fn data_readonly_outcome(&mut self, c: &mut DataSmoke) -> Poll {
        let n = self.downloads.started();
        let Some(e) = self.data_first() else {
            return Pending;
        };
        if n == c.before {
            // no entry yet: either the toast ("cannot download") and no entry, or still coming
            return if c.runs > 400 {
                c.notes
                    .push("a download into a read-only folder lists nothing".into());
                Pass
            } else {
                Pending
            };
        }
        match &e.state {
            DownloadState::Running => Pending,
            DownloadState::Complete => Fail(format!(
                "a download into a read-only folder completed at {}",
                e.path.display()
            )),
            other => {
                let detail = e.detail();
                c.notes.push(format!(
                    "a download into a read-only folder reads {detail:?}"
                ));
                expect(!detail.contains("interrupted ("), || {
                    format!("the row says {detail:?}: a reason number, not words ({other:?})")
                })
            }
        }
    }

    fn data_open_and_reveal(&mut self, c: &mut DataSmoke) {
        // Open and Show in folder start programs: only with the stand-in opener first on the
        // path and no session bus to reach a file manager through
        let stub_first = std::env::var_os("PATH").is_some_and(|p| {
            std::env::split_paths(&p).next() == c.opener_log.parent().map(|d| d.join("bin"))
        });
        let no_bus =
            std::env::var("DBUS_SESSION_BUS_ADDRESS").is_ok_and(|a| a == data_smoke::NO_BUS);
        if !stub_first || !no_bus {
            c.download = None;
            c.notes.push(
                "Open and Show in folder skipped: not run under --smoke data's parent".into(),
            );
            return;
        }
        let _ = std::fs::remove_file(&c.opener_log);
        let done = self
            .downloads
            .entries()
            .iter()
            .find(|e| e.state == DownloadState::Complete && e.path.exists())
            .map(|e| e.id);
        c.download = done;
        if let Some(id) = done {
            self.download_action(id, DownloadAction::Open);
            self.download_action(id, DownloadAction::Reveal);
        }
    }

    fn data_opener_asked(&self, c: &DataSmoke) -> Poll {
        if c.download.is_none() {
            return if c
                .notes
                .last()
                .is_some_and(|n| n.starts_with("Open and Show"))
            {
                Pass
            } else {
                Fail("no finished download with a file to open".into())
            };
        }
        let log = std::fs::read_to_string(&c.opener_log).unwrap_or_default();
        if log.lines().count() >= 2 {
            Pass
        } else if c.runs > 400 {
            Fail(format!("the opener was asked {:?}", log))
        } else {
            Pending
        }
    }

    fn data_open_missing(&mut self, c: &mut DataSmoke) {
        let _ = std::fs::remove_file(&c.opener_log);
        let missing = self
            .downloads
            .entries()
            .iter()
            .find(|e| e.state == DownloadState::Complete)
            .map(|e| (e.id, e.path.clone()));
        if let Some((id, path)) = missing {
            let _ = std::fs::remove_file(path);
            self.download_action(id, DownloadAction::Open);
        }
    }

    fn data_opener_not_asked(&self, c: &DataSmoke) -> Poll {
        if c.runs < 150 {
            return Pending;
        }
        let log = std::fs::read_to_string(&c.opener_log).unwrap_or_default();
        expect(log.is_empty(), || {
            format!("a missing file was opened: {log:?}")
        })
    }

    fn data_downloads_all_active(&self) -> Poll {
        let left = self
            .downloads
            .entries()
            .iter()
            .filter(|e| !e.state.is_active())
            .count();
        when(left == 0)
    }

    // ---- archive

    fn data_archive_setting(&mut self, _: &mut DataSmoke) {
        ui(self, |w| {
            w.invoke_setting_changed("archive.after".into(), "12h".into())
        });
    }

    fn data_tab_titled(&self, title: &str) -> Option<TabId> {
        self.model
            .tabs
            .values()
            .find(|t| t.title == title)
            .map(|t| t.id)
    }

    fn data_age_and_archive(&mut self, c: &mut DataSmoke) {
        // the first "Page One" tab is hidden (the second is on top): it went unvisited 13 hours
        if let Some(tab) = self.data_tab_titled("Page One") {
            if let Some(t) = self.model.tabs.get_mut(&tab) {
                t.last_active_ms = now_ms() - 13 * 3_600_000;
            }
            c.victim = Some(tab);
        }
        self.archive_tick(now_ms());
    }

    fn data_archived(&mut self, c: &DataSmoke) -> Poll {
        self.archive_tick(now_ms());
        match c.victim {
            Some(tab) => when(!self.model.tabs.contains_key(&tab)),
            None => Fail("no tab to put away".into()),
        }
    }

    fn data_archive_rows(&self, rows: usize) -> Poll {
        let Some(w) = self.w.window.upgrade() else {
            return Pending;
        };
        when(
            self.shown_page() == Some(InternalPage::Archive)
                && w.get_archive_lines().row_count() == rows,
        )
    }

    fn data_archive_key(&self) -> Option<i32> {
        let w = self.w.window.upgrade()?;
        w.get_archive_lines()
            .iter()
            .find(|l| !l.header)
            .map(|l| l.key)
    }

    fn data_restored(&mut self, c: &mut DataSmoke) -> Poll {
        let back = self
            .model
            .tabs
            .values()
            .filter(|t| t.url.ends_with("/page") && t.title == "Page One")
            .count();
        // the original is gone: one tab on /page, the restored one
        c.before = back;
        let empty = self
            .w
            .window
            .upgrade()
            .is_some_and(|w| w.get_archive_lines().row_count() == 0);
        when(back >= 1 && empty)
    }

    fn data_age_and_archive_restored(&mut self, c: &mut DataSmoke) {
        // the Archive page is shown, so the restored tab is hidden and can age
        if let Some(tab) = self
            .model
            .tabs
            .values()
            .find(|t| t.url.ends_with("/page") && t.title == "Page One")
            .map(|t| t.id)
        {
            if let Some(t) = self.model.tabs.get_mut(&tab) {
                t.last_active_ms = now_ms() - 13 * 3_600_000;
            }
            c.victim = Some(tab);
        }
        self.archive_tick(now_ms());
    }

    fn data_archived_again(&mut self, c: &DataSmoke) -> Poll {
        // the lifecycle tick's own call, again until the tab is idle and filed
        self.archive_tick(now_ms());
        match c.victim {
            Some(tab) if !self.model.tabs.contains_key(&tab) => self.data_archive_rows(2),
            Some(_) => Pending,
            None => Fail("no restored tab to put away".into()),
        }
    }

    // ---- tasks

    fn data_victim_ready(&self, c: &DataSmoke) -> Poll {
        when(
            c.victim
                .and_then(|t| self.model.tabs.get(&t))
                .is_some_and(|t| t.title == "Page One"),
        )
    }

    fn data_tasks_listed(&mut self, c: &mut DataSmoke) -> Poll {
        let Some(w) = self.w.window.upgrade() else {
            return Pending;
        };
        if self.shown_page() != Some(InternalPage::Tasks) {
            return Pending;
        }
        self.sample_tasks();
        let rows: Vec<_> = w.get_tasks().iter().collect();
        let tab_rows = rows.iter().filter(|r| r.tab >= 0).count();
        let procs: Vec<_> = rows.iter().filter(|r| r.tab < 0).collect();
        let with_pid = rows.iter().filter(|r| r.tab >= 0 && r.pid > 0).count();
        if procs.len() < 2 {
            return Pending;
        }
        if c.runs < 30 {
            // a second sample gives the CPU a baseline
            return Pending;
        }
        let described = rows
            .iter()
            .map(|r| format!("{}[{}|{}|{}|{}]", r.title, r.pid, r.state, r.memory, r.cpu))
            .collect::<Vec<_>>()
            .join("; ");
        c.notes.push(format!(
            "task manager rows ({tab_rows} tab rows, {with_pid} with a process): {described}"
        ));
        let bad: Vec<_> = procs.iter().filter(|r| r.memory.is_empty()).collect();
        if !bad.is_empty() {
            return Fail(format!("process rows without memory: {bad:?}"));
        }
        // a tab on a web page runs in a renderer of wubble's: its row has that process's numbers
        let blank: Vec<_> = rows
            .iter()
            .filter(|r| r.tab >= 0 && r.pid > 0 && r.memory.is_empty())
            .map(|r| format!("{} (pid {})", r.title, r.pid))
            .collect();
        expect(blank.is_empty(), || {
            format!("tab rows with no process numbers: {blank:?}")
        })
    }

    /// "End" on the process row of the newest renderer (the process rows are what the page
    /// can end where a tab's own renderer is not known).
    fn data_end_victim(&mut self, c: &mut DataSmoke) {
        c.before = 0;
        let Some(w) = self.w.window.upgrade() else {
            return;
        };
        let newest = w
            .get_tasks()
            .iter()
            .filter(|r| r.tab < 0 && r.can_end && matches!(r.title.as_str(), "Renderer" | "Zygote"))
            .max_by_key(|r| r.pid);
        match newest {
            Some(r) => {
                c.before = r.pid as usize;
                let pid = r.pid;
                ui(self, move |w| {
                    w.invoke_task_action(-1, pid, TaskAction::End)
                });
            }
            None => c.notes.push("no renderer process row to end".into()),
        }
    }

    fn data_victim_ended(&mut self, c: &mut DataSmoke) -> Poll {
        let Some(w) = self.w.window.upgrade() else {
            return Pending;
        };
        let pid = c.before as i32;
        if pid == 0 {
            return Fail("no renderer process row to end".into());
        }
        // the row goes (its process is gone), whatever the tab then shows
        self.sample_tasks();
        let present = w.get_tasks().iter().any(|r| r.pid == pid);
        if present && c.runs < 800 {
            return Pending;
        }
        let tab = c.victim.and_then(|t| self.model.tabs.get(&t));
        c.notes.push(format!(
            "after ending pid {pid}: process listed={present}, tab present={}, title={:?}, live={:?}",
            tab.is_some(),
            tab.map(|t| t.title.clone()),
            tab.map(|t| t.lifecycle.liveness)
        ));
        expect(!present, || format!("pid {pid} is still listed after End"))
    }

    // ---- agent activity

    fn data_log_call(&mut self, tool: &str) {
        use wubble_core::agent::SessionId;
        use wubble_core::agent::log::Outcome;
        // the host's own path for an agent's call
        self.agent_log(
            &SessionId("smoke".into()),
            tool,
            Some("http://127.0.0.1/".into()),
            Outcome::Done,
        );
    }

    fn data_agent_rows(&self, rows: usize) -> Poll {
        let Some(w) = self.w.window.upgrade() else {
            return Pending;
        };
        when(
            self.shown_page() == Some(InternalPage::AgentLog)
                && w.get_agent_log().row_count() == rows,
        )
    }

    // ---- the end

    /// Each link of Settings' Pages group, pressed through the shell's action: its page shows.
    fn data_links(&mut self, c: &mut DataSmoke) -> Poll {
        let links = &wubble_core::pages::PAGE_LINKS;
        let Some(link) = links.get(c.before) else {
            c.before = 0;
            c.key = 0;
            return Pass;
        };
        if c.key == 0 {
            c.key = 1;
            let id = link.action.id();
            ui(self, move |w| w.invoke_shell_action(id.into()));
            return Pending;
        }
        if self.shown_page() == Some(link.page) {
            c.before += 1;
            c.key = 0;
        }
        Pending
    }

    fn data_summary(&mut self, c: &mut DataSmoke) {
        let Some(w) = self.w.window.upgrade() else {
            return;
        };
        let mut lines = vec![];
        // the pages' own models: history, bookmarks, downloads
        for l in w.get_history_lines().iter().filter(|l| !l.header) {
            lines.push(format!("history {}|{}", l.url, l.title));
        }
        for l in w.get_bookmark_lines().iter() {
            lines.push(format!(
                "bookmark {}|{}",
                l.title,
                if l.is_folder {
                    "(folder)".into()
                } else {
                    l.url.to_string()
                }
            ));
        }
        for d in self.downloads.entries() {
            lines.push(format!("download {}|{}", d.name, d.state.as_str()));
        }
        let archived = w.get_archive_lines().iter().filter(|l| !l.header).count();
        c.notes.push(format!("archive rows at the end: {archived}"));
        let path = self.data_profile().join(data_smoke::SUMMARY);
        if let Err(e) = std::fs::write(&path, lines.join("\n")) {
            self.fail(format!("cannot write {}: {e}", path.display()));
        }
    }
}
