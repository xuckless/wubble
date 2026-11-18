//! Favicon images, fetched by CEF itself (`CefBrowserHost::DownloadImage`), so
//! the request goes through the browser's own context, proxy, cache and
//! request filter and no HTTP client or image codec exists in wubble.
//!
//! Idiom: cefclient `ClientHandler::OnFaviconURLChange` +
//! `ClientDownloadImageCallback` and `root_window_*::OnSetFavicon`
//! (cef `tests/cefclient/browser/client_handler.cc`, BSD-3-Clause), reading
//! the image with `GetAsBitmap` as RGBA with straight alpha.
//!
//! `is_favicon` is true, so no cookies are sent or stored (Chrome's rule): a
//! deliberate deviation from "the page's cookies apply", recorded in TODO.md.

use std::sync::{Arc, Mutex};

use wubble_core::favicon::valid_rgba;

use cef::{
    AlphaType, ColorType, DownloadImageCallback, ImplBinaryValue, ImplDownloadImageCallback,
    ImplImage, WrapDownloadImageCallback, rc::Rc,
};

pub use wubble_core::favicon::MAX_ICON_PX;

/// A page icon as raw pixels: straight (non-premultiplied) RGBA, row-major,
/// `rgba.len() == width * height * 4`, both sides in `1..=MAX_ICON_PX`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaviconImage {
    /// The icon URL the page declared.
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The pixels CEF handed back, if they are a usable icon: both sides in
/// `1..=MAX_ICON_PX` and exactly `width * height * 4` bytes. Nothing is
/// resampled; an icon CEF could not shrink is dropped.
pub(crate) fn accept(url: &str, width: i32, height: i32, rgba: Vec<u8>) -> Option<FaviconImage> {
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    valid_rgba(width, height, rgba.len()).then(|| FaviconImage {
        url: url.to_string(),
        width,
        height,
        rgba,
    })
}

/// Images finished downloading, taken by `View::drain_events`.
pub(crate) type Sink = Arc<Mutex<Vec<FaviconImage>>>;

cef::wrap_download_image_callback! {
    pub(crate) struct FaviconCallback {
        sink: Sink,
    }

    impl DownloadImageCallback {
        fn on_download_image_finished(
            &self,
            image_url: Option<&cef::CefString>,
            _http_status_code: ::std::os::raw::c_int,
            image: Option<&mut cef::Image>,
        ) {
            let url = image_url.map(|u| u.to_string()).unwrap_or_default();
            let Some(image) = image else {
                eprintln!("wubble-engine: favicon {url}: download failed");
                return;
            };
            let (mut width, mut height) = (0, 0);
            let bitmap = image.as_bitmap(
                1.0,
                ColorType::RGBA_8888,
                AlphaType::POSTMULTIPLIED,
                Some(&mut width),
                Some(&mut height),
            );
            // `data` copies into the Vec's length, so it must be sized first.
            let Some(bitmap) = bitmap else { return };
            let mut bytes = vec![0u8; bitmap.size()];
            if bytes.is_empty() || bitmap.data(Some(&mut bytes), 0) != bytes.len() {
                return;
            }
            match accept(&url, width, height, bytes) {
                Some(icon) => self.sink.lock().unwrap_or_else(|e| e.into_inner()).push(icon),
                None => eprintln!("wubble-engine: favicon {url}: unusable size {width}x{height}"),
            }
        }
    }
}

/// Starts the download; the result lands in `sink`.
pub(crate) fn fetch(host: &impl cef::ImplBrowserHost, url: &str, sink: &Sink) {
    let mut callback = FaviconCallback::new(sink.clone());
    host.download_image(Some(&url.into()), 1, MAX_ICON_PX, 0, Some(&mut callback));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exact_sizes_only() {
        let ok = accept("u", 2, 3, vec![0; 24]).unwrap();
        assert_eq!((ok.width, ok.height, ok.rgba.len()), (2, 3, 24));
        assert!(accept("u", 64, 64, vec![0; 64 * 64 * 4]).is_some());
        assert!(accept("u", 0, 3, vec![]).is_none());
        assert!(accept("u", -1, 3, vec![0; 12]).is_none());
        assert!(accept("u", 65, 1, vec![0; 65 * 4]).is_none());
        assert!(accept("u", 2, 3, vec![0; 23]).is_none());
        assert!(accept("u", 2, 3, vec![]).is_none());
    }
}
