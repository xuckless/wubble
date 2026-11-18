//! Content filtering in the request path, with no CEF types in its API.
//!
//! The host implements [`ContentFilter`] and hands it to the engine through
//! [`crate::app::EngineConfig::content_filter`]. Every request a view is
//! about to send goes through [`ContentFilter::check`] on CEF's IO thread
//! (CEF `CefResourceRequestHandler::OnBeforeResourceLoad`, through welding's
//! `RequestFilter`), so the filter must answer from shared read-only data
//! and never wait on the UI thread. When a main-frame load commits, the view
//! asks [`ContentFilter::page_script`] on the UI thread and runs the answer
//! in the page (cosmetic filtering).
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
}

/// The host's content filter.
pub trait ContentFilter: Send + Sync {
    /// Decides one request. Called on CEF's IO thread; must not panic (the
    /// release profile aborts on panic).
    fn check(&self, request: &RequestInfo<'_>) -> RequestAction;

    /// Script to run in a page whose main-frame load just committed
    /// (`None`: nothing to inject). Called on the UI thread.
    fn page_script(&self, top_url: &str) -> Option<String>;
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

/// The kind of CEF's raw `cef_resource_type_t`; unknown values are `Other`.
pub fn kind_of(cef_type: u32) -> RequestKind {
    KINDS
        .iter()
        .find(|(t, _)| t.get_raw() == cef_type)
        .map_or(RequestKind::Other, |(_, kind)| *kind)
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
        let raw: Vec<u32> = KINDS.iter().map(|(t, _)| t.get_raw()).collect();
        let unique: HashSet<u32> = raw.iter().copied().collect();
        assert_eq!(unique.len(), raw.len(), "a CEF type is listed twice");
        let num = cef::ResourceType::NUM_VALUES.get_raw();
        assert!(raw.iter().all(|&r| r < num));
        // cef_resource_type_t lists 20 named types below RT_NUM_VALUES.
        assert_eq!(raw.len(), 20);
    }

    #[test]
    fn kinds_follow_the_table() {
        use cef::ResourceType as T;
        let k = |t: T| kind_of(t.get_raw());
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
        fn page_script(&self, _: &str) -> Option<String> {
            None
        }
    }

    #[test]
    fn decisions_reach_welding() {
        let filter = request_filter(Arc::new(BlockAds));
        let check = |url: &str| {
            filter.check(&welding::ResourceRequest {
                url: url.into(),
                resource_type: cef::ResourceType::SCRIPT.get_raw(),
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
