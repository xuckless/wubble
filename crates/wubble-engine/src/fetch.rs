//! Requests wubble makes itself through the page's own network stack: a manifest, a key or a
//! segment of a media download, fetched with CEF's `CefURLRequest` in a tab's request context,
//! so the site sees the tab's session (cookies with `UR_FLAG_ALLOW_STORED_CREDENTIALS`, the
//! same proxy and cache, the page as referrer). No CEF type leaves this module: the host gets
//! [`FetchEvent`]s.
//!
//! Pattern from cefclient `tests/cefclient/browser/urlrequest_test.cc` (`CefURLRequest::Create`
//! with a `CefURLRequestClient` that collects `OnDownloadData` and finishes in
//! `OnRequestComplete`) and CEF `include/cef_urlrequest.h` (chromiumembedded/cef,
//! BSD-3-Clause); no code copied. Callbacks arrive on the thread that created the request (the
//! UI thread, where the engine runs); each chunk is copied once into the event queue.

use std::collections::HashMap;
use std::os::raw::c_int;
use std::sync::{Arc, Mutex};

use cef::rc::Rc as _;
use cef::{
    ImplBrowser, ImplBrowserHost, ImplPostData, ImplPostDataElement, ImplRequest, ImplResponse,
    ImplUrlrequest, ImplUrlrequestClient, UrlrequestClient, WrapUrlrequestClient,
};

/// One request's identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FetchId(pub u64);

/// What to fetch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FetchSpec {
    /// The view whose request context (cookies, cache, proxy) the request uses.
    pub browser_id: i32,
    pub url: String,
    /// Sent as the referrer (the page the media plays on); empty for none.
    pub referrer: String,
    /// `Range: bytes=start-end` (inclusive end); the answer must then be 206.
    pub range: Option<(u64, u64)>,
    pub headers: Vec<(String, String)>,
    /// POST this body instead of a GET.
    pub post: Option<Vec<u8>>,
    /// Send and store the context's cookies (`UR_FLAG_ALLOW_STORED_CREDENTIALS`).
    pub credentials: bool,
    /// May be answered from the HTTP cache (a manifest the page has just loaded); otherwise
    /// the cache is skipped (`UR_FLAG_SKIP_CACHE`).
    pub cached: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchEvent {
    /// A chunk of the body, in order.
    Data { id: FetchId, bytes: Vec<u8> },
    /// The request ended: every chunk was delivered (`Ok`, with the HTTP status), or it
    /// failed (a network error, an HTTP error status, a range the server ignored, or a
    /// cancel). After a failure the chunks delivered are not the whole body.
    Done {
        id: FetchId,
        result: Result<i32, String>,
    },
}

/// `cef_urlrequest_flags_t` values used here.
const FLAG_SKIP_CACHE: c_int = cef::sys::cef_urlrequest_flags_t::UR_FLAG_SKIP_CACHE.0 as c_int;
const FLAG_ALLOW_STORED_CREDENTIALS: c_int =
    cef::sys::cef_urlrequest_flags_t::UR_FLAG_ALLOW_STORED_CREDENTIALS.0 as c_int;

/// Whether an answer is the body asked for: 2xx, and 206 when a range was asked.
pub fn accepted(status: i32, ranged: bool) -> Result<(), String> {
    match (status, ranged) {
        (206, _) => Ok(()),
        (200..=299, false) => Ok(()),
        (200..=299, true) => Err("the server ignored the byte range".into()),
        (s, _) => Err(format!("HTTP {s}")),
    }
}

/// The verdict on a request's answer, once its first bytes (or its end) arrive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Verdict {
    #[default]
    Unknown,
    Accepted(i32),
    Refused(i32),
}

type Queue = Arc<Mutex<Vec<FetchEvent>>>;

cef::wrap_urlrequest_client! {
    struct FetchClient {
        id: FetchId,
        ranged: bool,
        queue: Queue,
        verdict: Arc<Mutex<Verdict>>,
    }

    impl UrlrequestClient {
        fn on_download_data(&self, request: Option<&mut cef::Urlrequest>, data: *const u8, data_length: usize) {
            let Some(request) = request else { return };
            let accepted = {
                let mut verdict = self.verdict.lock().unwrap_or_else(|e| e.into_inner());
                if *verdict == Verdict::Unknown {
                    let status = request.response().map_or(0, |r| r.status());
                    *verdict = match accepted(status, self.ranged) {
                        Ok(()) => Verdict::Accepted(status),
                        Err(_) => Verdict::Refused(status),
                    };
                }
                matches!(*verdict, Verdict::Accepted(_))
            };
            if !accepted {
                // An error page or a whole file instead of a range: not the body asked for.
                request.cancel();
                return;
            }
            if data.is_null() || data_length == 0 {
                return;
            }
            // SAFETY: CEF hands `data_length` readable bytes valid for this call.
            let bytes = unsafe { std::slice::from_raw_parts(data, data_length) }.to_vec();
            self.queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(FetchEvent::Data { id: self.id, bytes });
        }

        fn on_request_complete(&self, request: Option<&mut cef::Urlrequest>) {
            let Some(request) = request else { return };
            let status = request.response().map_or(0, |r| r.status());
            let verdict = *self.verdict.lock().unwrap_or_else(|e| e.into_inner());
            let result = match verdict {
                Verdict::Refused(s) => accepted(s, self.ranged).map(|()| s),
                _ if request.request_status() == cef::UrlrequestStatus::SUCCESS => {
                    accepted(status, self.ranged).map(|()| status)
                }
                _ if request.request_status() == cef::UrlrequestStatus::CANCELED => Err("cancelled".into()),
                _ => Err(format!("network error {}", *request.request_error().as_ref() as i32)),
            };
            self.queue
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(FetchEvent::Done { id: self.id, result });
        }
    }
}

/// The running requests. Owned by the host on the UI thread.
#[derive(Default)]
pub struct Fetcher {
    next: u64,
    running: HashMap<FetchId, cef::Urlrequest>,
    queue: Queue,
}

impl Fetcher {
    /// Starts `spec`; its events arrive through [`Self::take_events`]. `Err` when the view is
    /// gone or CEF refuses the request.
    pub fn start(&mut self, spec: &FetchSpec) -> Result<FetchId, String> {
        let browser = cef::browser_host_get_browser_by_identifier(spec.browser_id)
            .ok_or_else(|| "the page is gone".to_owned())?;
        let mut context = browser.host().and_then(|h| h.request_context());
        let mut request = cef::request_create().ok_or("CEF made no request")?;
        request.set_url(Some(&cef::CefString::from(spec.url.as_str())));
        if !spec.referrer.is_empty() {
            request.set_referrer(
                Some(&cef::CefString::from(spec.referrer.as_str())),
                cef::ReferrerPolicy::CLEAR_REFERRER_ON_TRANSITION_FROM_SECURE_TO_INSECURE,
            );
        }
        for (name, value) in &spec.headers {
            request.set_header_by_name(
                Some(&cef::CefString::from(name.as_str())),
                Some(&cef::CefString::from(value.as_str())),
                1,
            );
        }
        if let Some((start, end)) = spec.range {
            request.set_header_by_name(
                Some(&cef::CefString::from("Range")),
                Some(&cef::CefString::from(
                    format!("bytes={start}-{end}").as_str(),
                )),
                1,
            );
        }
        if let Some(body) = &spec.post {
            let mut data = cef::post_data_create().ok_or("CEF made no post data")?;
            let mut element = cef::post_data_element_create().ok_or("CEF made no post data")?;
            element.set_to_bytes(body.len(), body.as_ptr());
            data.add_element(Some(&mut element));
            request.set_method(Some(&cef::CefString::from("POST")));
            request.set_post_data(Some(&mut data));
        }
        let flags = if spec.cached { 0 } else { FLAG_SKIP_CACHE }
            | if spec.credentials {
                FLAG_ALLOW_STORED_CREDENTIALS
            } else {
                0
            };
        request.set_flags(flags);
        let id = FetchId(self.next);
        self.next += 1;
        let mut client = FetchClient::new(
            id,
            spec.range.is_some(),
            self.queue.clone(),
            Arc::new(Mutex::new(Verdict::default())),
        );
        let started =
            cef::urlrequest_create(Some(&mut request), Some(&mut client), context.as_mut())
                .ok_or_else(|| "CEF refused the request".to_owned())?;
        self.running.insert(id, started);
        Ok(id)
    }

    /// Stops a request; its `Done` (an error) still arrives.
    pub fn cancel(&mut self, id: FetchId) {
        if let Some(request) = self.running.get(&id) {
            request.cancel();
        }
    }

    /// Stops every request and releases its handle now, with what is queued: at shutdown, so
    /// no CEF reference outlives CEF. No event of these requests is reported.
    pub fn stop_all(&mut self) {
        for (_, request) in self.running.drain() {
            request.cancel();
        }
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// What arrived since the last call. A request's handle is released once its `Done` is
    /// taken.
    pub fn take_events(&mut self) -> Vec<FetchEvent> {
        let events = std::mem::take(&mut *self.queue.lock().unwrap_or_else(|e| e.into_inner()));
        for e in &events {
            if let FetchEvent::Done { id, .. } = e {
                self.running.remove(id);
            }
        }
        events
    }

    pub fn is_idle(&self) -> bool {
        self.running.is_empty()
    }
}

impl std::fmt::Debug for Fetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fetcher")
            .field("running", &self.running.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_must_be_the_body_asked_for() {
        assert_eq!(accepted(200, false), Ok(()));
        assert_eq!(accepted(204, false), Ok(()));
        assert_eq!(accepted(206, true), Ok(()));
        assert_eq!(accepted(206, false), Ok(()));
        assert!(accepted(200, true).unwrap_err().contains("range"));
        assert_eq!(accepted(404, false), Err("HTTP 404".into()));
        assert_eq!(accepted(0, false), Err("HTTP 0".into()));
    }

    #[test]
    fn flags_are_cefs() {
        assert_eq!((FLAG_SKIP_CACHE, FLAG_ALLOW_STORED_CREDENTIALS), (1, 8));
    }
}
