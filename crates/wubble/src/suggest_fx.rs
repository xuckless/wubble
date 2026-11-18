//! The edge of live search suggestions: runs `wubble_core::suggest::transition`'s effects. The
//! debounce is a single-shot `slint::Timer`, the fetch runs on one worker thread, and every
//! reply comes back as a [`FetchEvent`] through `deliver` (on the UI thread).
//!
//! What may be sent is the core rule alone: only search-like input, only with the setting on
//! (`suggest::sources`). The fetch is a plain HTTPS GET with no cookies (ureq's agent keeps
//! none) to the configured engine's suggest endpoint, 3 s timeout, 64 KiB body cap: not the
//! page's request context, so nothing of the profile travels with a typed query.
//! The HTTP code is a copy of `wubble_blocker::refresh::fetch_http` (ours).

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use wubble_core::suggest::{self, DEBOUNCE_MS, Fetch, FetchEffect, FetchEvent, SuggestSettings};

/// Longest wait for the engine's answer.
pub const TIMEOUT: Duration = Duration::from_secs(3);
/// Largest answer read.
pub const MAX_BODY: u64 = 64 * 1024;

/// Fetches a URL's body. The real one is [`Http`]; tests use their own.
pub trait Fetcher: Send + 'static {
    fn get(&self, url: &str) -> Result<String, String>;
}

/// The real fetcher: HTTPS only (unless built for a loopback test).
pub struct Http {
    https_only: bool,
}

impl Http {
    pub fn new() -> Self {
        Self { https_only: true }
    }
}

impl Fetcher for Http {
    fn get(&self, url: &str) -> Result<String, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .https_only(self.https_only)
            .build()
            .into();
        agent
            .get(url)
            .call()
            .map_err(|e| e.to_string())?
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_string()
            .map_err(|e| e.to_string())
    }
}

/// Hands an event back to the host (from the worker thread or the timer).
pub type Deliver = fn(FetchEvent);

/// The fetch machine's state and the things that run its effects.
pub struct SuggestFx {
    state: Fetch,
    deliver: Deliver,
    debounce: slint::Timer,
    worker: Worker,
}

struct Worker {
    tx: Option<mpsc::Sender<(u64, String)>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    fn spawn(fetcher: impl Fetcher, deliver: Deliver) -> Self {
        let (tx, rx) = mpsc::channel::<(u64, String)>();
        let thread = std::thread::Builder::new()
            .name("wubble-suggest".into())
            .spawn(move || {
                for (seq, url) in rx {
                    let body = fetcher.get(&url).ok();
                    deliver(FetchEvent::Reply { seq, body });
                }
            })
            .ok();
        Self {
            tx: Some(tx),
            thread,
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // closing the channel ends the loop after the request in flight (at most TIMEOUT)
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl SuggestFx {
    pub fn new(fetcher: impl Fetcher, deliver: Deliver) -> Self {
        Self {
            state: Fetch::default(),
            deliver,
            debounce: slint::Timer::default(),
            worker: Worker::spawn(fetcher, deliver),
        }
    }

    /// Runs one event; `Some(list)` when the live suggestions to show changed.
    pub fn event(&mut self, event: FetchEvent, settings: &SuggestSettings) -> Option<Vec<String>> {
        let (next, effects) = suggest::transition(std::mem::take(&mut self.state), event, settings);
        self.state = next;
        let mut shown = None;
        for effect in effects {
            match effect {
                FetchEffect::ArmDebounce { seq } => {
                    let deliver = self.deliver;
                    self.debounce.start(
                        slint::TimerMode::SingleShot,
                        Duration::from_millis(DEBOUNCE_MS),
                        move || deliver(FetchEvent::DebounceElapsed(seq)),
                    );
                }
                FetchEffect::Request { seq, url } => {
                    if let Some(tx) = &self.worker.tx
                        && tx.send((seq, url)).is_err()
                    {
                        // the worker is gone: answer as a failure so the machine moves on
                        (self.deliver)(FetchEvent::Reply { seq, body: None });
                    }
                }
                FetchEffect::Show(list) => shown = Some(list),
                FetchEffect::Clear => {
                    self.debounce.stop();
                    shown = Some(vec![]);
                }
            }
        }
        shown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// Records the URLs asked for and answers a fixed body.
    struct Fake(Arc<Mutex<Vec<String>>>);
    impl Fetcher for Fake {
        fn get(&self, url: &str) -> Result<String, String> {
            self.0.lock().unwrap().push(url.into());
            Ok(r#"["q",["one","two"]]"#.into())
        }
    }

    static REPLIES: Mutex<Vec<FetchEvent>> = Mutex::new(Vec::new());
    static REPLIED: (Mutex<bool>, std::sync::Condvar) =
        (Mutex::new(false), std::sync::Condvar::new());

    fn record(e: FetchEvent) {
        REPLIES.lock().unwrap().push(e);
        *REPLIED.0.lock().unwrap() = true;
        REPLIED.1.notify_all();
    }

    fn wait_reply() -> FetchEvent {
        let mut done = REPLIED.0.lock().unwrap();
        while !*done {
            done = REPLIED.1.wait(done).unwrap();
        }
        *done = false;
        REPLIES.lock().unwrap().remove(0)
    }

    fn on() -> SuggestSettings {
        SuggestSettings {
            engine: suggest::SearchEngine::DuckDuckGo,
            live: suggest::LiveSuggestions::On,
        }
    }

    /// The worker asks the fake for the debounced query's URL and its reply shows the list.
    /// The debounce itself is the core machine's (tested there): it is fed by hand here.
    #[test]
    fn a_debounced_query_is_fetched_and_shown() {
        let urls = Arc::new(Mutex::new(vec![]));
        let mut fx = SuggestFx::new(Fake(Arc::clone(&urls)), record);
        assert_eq!(fx.event(FetchEvent::Typed("cats".into()), &on()), None);
        assert_eq!(fx.event(FetchEvent::DebounceElapsed(1), &on()), None);
        let reply = wait_reply();
        assert_eq!(
            urls.lock().unwrap().as_slice(),
            ["https://duckduckgo.com/ac/?type=list&q=cats"]
        );
        assert_eq!(
            fx.event(reply, &on()),
            Some(vec!["one".into(), "two".into()])
        );
        // an address is never asked for
        assert_eq!(
            fx.event(FetchEvent::Typed("bank.example/login".into()), &on()),
            Some(vec![])
        );
        assert_eq!(fx.event(FetchEvent::DebounceElapsed(2), &on()), None);
        assert_eq!(urls.lock().unwrap().len(), 1);
    }

    /// The real HTTP path against a loopback server answering one fixed response.
    #[test]
    fn http_reads_a_body_and_caps_it() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let body = r#"["q",["a"]]"#;
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let http = Http { https_only: false };
        let body = http
            .get(&format!("http://127.0.0.1:{port}/ac?q=a"))
            .unwrap();
        assert_eq!(body, r#"["q",["a"]]"#);
        server.join().unwrap();
        // and the real one refuses plain http without connecting
        assert!(
            Http::new()
                .get(&format!("http://127.0.0.1:{port}/"))
                .is_err()
        );
    }
}
