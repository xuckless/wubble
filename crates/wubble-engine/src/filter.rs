//! Content filtering in the request path, with no CEF types in its API.
//!
//! The host implements [`ContentFilter`] and hands it to the engine through
//! [`crate::app::EngineConfig::content_filter`]. Every request a view is
//! about to send goes through [`ContentFilter::check`] on CEF's IO thread
//! (CEF `CefResourceRequestHandler::OnBeforeResourceLoad`, through welding's
//! `RequestFilter`), so the filter must answer from shared read-only data
//! and never wait on the UI thread. Cosmetic filtering goes through the
//! frames' own scripts: each frame's `cosmetic.js` asks, on the UI thread,
//! for [`ContentFilter::frame_cosmetic`] when its context is created and for
//! [`ContentFilter::generic_hide`] as new class names and ids appear.
//!
//! Hook shape from cefclient `ClientHandler::GetResourceRequestHandler` /
//! `OnBeforeResourceLoad` (chromiumembedded/cef, BSD-3-Clause) and Brave's
//! shields (brave-core `components/brave_shields`, MPL-2.0); no code copied.

use std::fmt;
use std::sync::Arc;

/// What a request loads, neutral to CEF.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestKind {
    Document,
    Subdocument,
    Stylesheet,
    Script,
    Image,
    Font,
    Object,
    Media,
    Xhr,
    Ping,
    Other,
}

/// One request about to be sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestInfo<'a> {
    pub url: &'a str,
    /// The top-level page the request belongs to (the request itself for a
    /// [`RequestKind::Document`]); decides first- vs third-party. May be empty.
    pub top_url: &'a str,
    pub kind: RequestKind,
    /// The view's [`crate::view::View::browser_id`]; 0 for requests without a
    /// browser (service workers).
    pub browser_id: i32,
}

/// What to do with a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestAction {
    Allow,
    Block,
    /// Load this URL instead.
    Redirect(String),
    /// Answer with the content of this base64 `data:` URL instead of
    /// sending the request (a blocker's stub resource); the request is
    /// cancelled if it does not parse.
    Serve(String),
}

/// The host's content filter.
pub trait ContentFilter: Send + Sync {
    /// Decides one request. Called on CEF's IO thread; must not panic (the
    /// release profile aborts on panic).
    fn check(&self, request: &RequestInfo<'_>) -> RequestAction;

    /// What a frame whose script context was just created is given. `top_url`
    /// is the page the frame belongs to (the shields decision), `frame_url`
    /// the frame's own (its host's rules). Called on the UI thread.
    fn frame_cosmetic(&self, top_url: &str, frame_url: &str) -> FrameCosmetic;

    /// Stylesheet rules hiding the generic selectors that match class names
    /// and ids a frame has found. Called on the UI thread.
    fn generic_hide(
        &self,
        top_url: &str,
        frame_url: &str,
        classes: &[String],
        ids: &[String],
    ) -> Vec<String>;
}

/// What [`ContentFilter::frame_cosmetic`] gives a frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameCosmetic {
    /// Stylesheet rules (hiding).
    pub css: Vec<String>,
    /// Procedural filters (adblock's JSON) for the frame's executor.
    pub procedural: Vec<String>,
    /// Scriptlets, run in the frame from the browser (a page's CSP does not apply).
    pub script: Option<String>,
    /// The frame should report class names and ids ([`ContentFilter::generic_hide`]).
    pub generic: bool,
}

impl fmt::Debug for dyn ContentFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ContentFilter")
    }
}

/// The one CEF resource type -> [`RequestKind`] table. Brave maps
/// Chromium's own types the same way (brave-core
/// `ad_block_request_type_util.cc`, MPL-2.0): workers load scripts, favicons
/// are images, plugin loads are objects; CSP reports and prefetches have no
/// kind of their own here.
const KINDS: &[(cef::ResourceType, RequestKind)] = {
    use RequestKind as K;
    use cef::ResourceType as T;
    &[
        (T::MAIN_FRAME, K::Document),
        (T::NAVIGATION_PRELOAD_MAIN_FRAME, K::Document),
        (T::SUB_FRAME, K::Subdocument),
        (T::NAVIGATION_PRELOAD_SUB_FRAME, K::Subdocument),
        (T::STYLESHEET, K::Stylesheet),
        (T::SCRIPT, K::Script),
        (T::WORKER, K::Script),
        (T::SHARED_WORKER, K::Script),
        (T::SERVICE_WORKER, K::Script),
        (T::IMAGE, K::Image),
        (T::FAVICON, K::Image),
        (T::FONT_RESOURCE, K::Font),
        (T::OBJECT, K::Object),
        (T::PLUGIN_RESOURCE, K::Object),
        (T::MEDIA, K::Media),
        (T::XHR, K::Xhr),
        (T::PING, K::Ping),
        (T::CSP_REPORT, K::Other),
        (T::SUB_RESOURCE, K::Other),
        (T::PREFETCH, K::Other),
    ]
};

/// A resource type's raw value as welding carries it. CEF's enum raw type is
/// `i32` on Windows and `u32` elsewhere; the values are small and non-negative.
#[allow(clippy::unnecessary_cast)]
fn raw(t: cef::ResourceType) -> u32 {
    t.get_raw() as u32
}

/// The kind of CEF's raw `cef_resource_type_t`; unknown values are `Other`.
pub fn kind_of(cef_type: u32) -> RequestKind {
    KINDS
        .iter()
        .find(|(t, _)| raw(*t) == cef_type)
        .map_or(RequestKind::Other, |(_, kind)| *kind)
}

/// One response's headers, as the response hook saw them (CEF `OnResourceResponse`, through
/// welding's `ResponseObserver`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResponseInfo<'a> {
    pub url: &'a str,
    /// As [`RequestInfo::browser_id`].
    pub browser_id: i32,
    pub kind: RequestKind,
    pub status: i32,
    /// Lower case, without parameters.
    pub mime: &'a str,
    pub content_length: Option<u64>,
    /// The raw `Content-Range` header (empty when absent).
    pub content_range: &'a str,
    pub accepts_ranges: bool,
}

/// Sees every response a view gets (the media downloader's detection). Called on CEF's IO
/// thread: it must be cheap, never wait on the UI thread, and not panic.
pub trait ResponseObserver: Send + Sync {
    fn seen(&self, response: &ResponseInfo<'_>);
}

impl fmt::Debug for dyn ResponseObserver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ResponseObserver")
    }
}

/// welding's response observer for `observer`.
pub(crate) fn response_observer(observer: Arc<dyn ResponseObserver>) -> welding::ResponseObserver {
    welding::ResponseObserver::new(move |r| {
        observer.seen(&ResponseInfo {
            url: &r.url,
            browser_id: r.browser_id,
            kind: kind_of(r.resource_type),
            status: r.status,
            mime: &r.mime,
            content_length: r.content_length,
            content_range: &r.content_range,
            accepts_ranges: r.accepts_ranges,
        })
    })
}

/// welding's request filter for `filter`.
pub(crate) fn request_filter(filter: Arc<dyn ContentFilter>) -> welding::RequestFilter {
    welding::RequestFilter::new(move |request| {
        let info = RequestInfo {
            url: &request.url,
            top_url: &request.top_url,
            kind: kind_of(request.resource_type),
            browser_id: request.browser_id,
        };
        match filter.check(&info) {
            RequestAction::Allow => welding::RequestAction::Allow,
            RequestAction::Block => welding::RequestAction::Cancel,
            RequestAction::Redirect(url) => welding::RequestAction::Redirect(url),
            RequestAction::Serve(data_url) => welding::RequestAction::Serve(data_url),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Every value CEF defines (`0..RT_NUM_VALUES`, minus the gaps CEF left
    /// for removed types) is in the table exactly once.
    #[test]
    fn the_table_covers_every_cef_type_once() {
        let all: Vec<u32> = KINDS.iter().map(|(t, _)| raw(*t)).collect();
        let unique: HashSet<u32> = all.iter().copied().collect();
        assert_eq!(unique.len(), all.len(), "a CEF type is listed twice");
        let num = raw(cef::ResourceType::NUM_VALUES);
        assert!(all.iter().all(|&r| r < num));
        // cef_resource_type_t lists 20 named types below RT_NUM_VALUES.
        assert_eq!(all.len(), 20);
    }

    #[test]
    fn kinds_follow_the_table() {
        use cef::ResourceType as T;
        let k = |t: T| kind_of(raw(t));
        assert_eq!(k(T::MAIN_FRAME), RequestKind::Document);
        assert_eq!(k(T::SUB_FRAME), RequestKind::Subdocument);
        assert_eq!(k(T::SCRIPT), RequestKind::Script);
        assert_eq!(k(T::SERVICE_WORKER), RequestKind::Script);
        assert_eq!(k(T::FAVICON), RequestKind::Image);
        assert_eq!(k(T::XHR), RequestKind::Xhr);
        assert_eq!(k(T::PING), RequestKind::Ping);
        assert_eq!(k(T::PREFETCH), RequestKind::Other);
        assert_eq!(kind_of(9999), RequestKind::Other);
    }

    struct BlockAds;

    impl ContentFilter for BlockAds {
        fn check(&self, request: &RequestInfo<'_>) -> RequestAction {
            match request.url {
                "https://ads.test/x.js" => RequestAction::Block,
                "https://q.test/?utm=1" => RequestAction::Redirect("https://q.test/".into()),
                _ => RequestAction::Allow,
            }
        }
        fn frame_cosmetic(&self, _: &str, _: &str) -> FrameCosmetic {
            FrameCosmetic::default()
        }
        fn generic_hide(&self, _: &str, _: &str, _: &[String], _: &[String]) -> Vec<String> {
            Vec::new()
        }
    }

    #[test]
    fn decisions_reach_welding() {
        let filter = request_filter(Arc::new(BlockAds));
        let check = |url: &str| {
            filter.check(&welding::ResourceRequest {
                url: url.into(),
                resource_type: raw(cef::ResourceType::SCRIPT),
                ..Default::default()
            })
        };
        assert_eq!(
            check("https://ads.test/x.js"),
            welding::RequestAction::Cancel
        );
        assert_eq!(
            check("https://q.test/?utm=1"),
            welding::RequestAction::Redirect("https://q.test/".into())
        );
        assert_eq!(check("https://ok.test/"), welding::RequestAction::Allow);
    }
}
