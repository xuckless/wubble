//! Favicon images, fetched by CEF itself (`CefBrowserHost::DownloadImage`), so
//! the request goes through the browser's own context, proxy, cache and
//! request filter and no HTTP client or image codec exists in wubble.
//!
//! Idiom: cefclient `ClientHandler::OnFaviconURLChange` +
//! `ClientDownloadImageCallback` and `root_window_*::OnSetFavicon`
//! (cef `tests/cefclient/browser/client_handler.cc`, BSD-3-Clause), reading
//! the image with `GetAsBitmap` as RGBA with straight alpha.
//!
//! CEF passes the renderer `max_image_size` times the device scale as the
//! largest bitmap, so on a 2x display a 144 px icon comes back at 128 px (one
//! representation at scale 2). An ICO's frames above that bound are dropped
//! and the rest kept as representations; `as_bitmap(1.0)` picks the one
//! closest to `MAX_ICON_PX`. An SVG is rasterised at `MAX_ICON_PX` by the
//! renderer (blink `ImageDownloaderImpl`, `DecodeSVG` at the preferred
//! size). Whatever arrives larger is shrunk here (`fit_rgba`), never refused.
//! Each icon URL is fetched once per page load (`nav.rs`), so a failure is
//! logged once.
//!
//! `is_favicon` is true, so no cookies are sent or stored (Chrome's rule): a
//! deliberate deviation from "the page's cookies apply", recorded in TODO.md.

use std::sync::{Arc, Mutex};

use wubble_core::favicon::fit_rgba;

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

/// The pixels CEF handed back as an icon, shrunk to `MAX_ICON_PX` when
/// larger. `None` only for an empty image or a byte count that does not
/// match its size.
pub(crate) fn accept(url: &str, width: i32, height: i32, rgba: Vec<u8>) -> Option<FaviconImage> {
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    let (width, height, rgba) = fit_rgba(width, height, rgba)?;
    Some(FaviconImage {
        url: url.to_string(),
        width,
        height,
        rgba,
    })
}

/// An image as raw pixels at full size (Copy Image): straight RGBA,
/// row-major, `rgba.len() == width * height * 4`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// The largest image `View::download_image` hands over: 16 Mpx, 64 MiB of RGBA.
pub const MAX_IMAGE_PIXELS: u64 = 16 << 20;

/// The pixels of a full-size image, or why there are none: empty, too large
/// or a byte count that does not match its size.
pub(crate) fn accept_full(width: i32, height: i32, rgba: Vec<u8>) -> Result<Rgba, String> {
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err("no image".into());
    };
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || rgba.len() as u64 != pixels * 4 {
        return Err("no image".into());
    }
    if pixels > MAX_IMAGE_PIXELS {
        return Err("the image is too large".into());
    }
    Ok(Rgba {
        width,
        height,
        rgba,
    })
}

/// Images finished downloading, as the events they become, taken by
/// `View::drain_events`.
pub(crate) type Sink = Arc<Mutex<Vec<crate::page::PageEvent>>>;

/// What a download is for: the page icon (shrunk, no cookies) or Copy Image
/// (full size, the page's cookies).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    Icon,
    Full,
}

cef::wrap_download_image_callback! {
    pub(crate) struct FaviconCallback {
        sink: Sink,
        purpose: Purpose,
    }

    impl DownloadImageCallback {
        fn on_download_image_finished(
            &self,
            image_url: Option<&cef::CefString>,
            _http_status_code: ::std::os::raw::c_int,
            image: Option<&mut cef::Image>,
        ) {
            use crate::page::PageEvent;
            let url = image_url.map(|u| u.to_string()).unwrap_or_default();
            let read = image.map(|image| {
                let (mut width, mut height) = (0, 0);
                let bitmap = image.as_bitmap(
                    1.0,
                    ColorType::RGBA_8888,
                    AlphaType::POSTMULTIPLIED,
                    Some(&mut width),
                    Some(&mut height),
                );
                // `data` copies into the Vec's length, so it must be sized first.
                bitmap.map(|bitmap| {
                    let mut bytes = vec![0u8; bitmap.size()];
                    let read = bitmap.data(Some(&mut bytes), 0) == bytes.len();
                    (width, height, if read { bytes } else { Vec::new() })
                })
            });
            let event = match (self.purpose, read.flatten()) {
                (Purpose::Icon, None) => {
                    eprintln!("wubble-engine: favicon {url}: download failed");
                    return;
                }
                (Purpose::Icon, Some((width, height, bytes))) => {
                    match accept(&url, width, height, bytes) {
                        Some(icon) => PageEvent::FaviconImage(icon),
                        None => {
                            eprintln!("wubble-engine: favicon {url}: unreadable {width}x{height} bitmap");
                            return;
                        }
                    }
                }
                (Purpose::Full, None) => PageEvent::ImageDownloaded {
                    url,
                    image: Err("download failed".into()),
                },
                (Purpose::Full, Some((width, height, bytes))) => PageEvent::ImageDownloaded {
                    url,
                    image: accept_full(width, height, bytes),
                },
            };
            self.sink.lock().unwrap_or_else(|e| e.into_inner()).push(event);
        }
    }
}

/// Starts the download; the result lands in `sink`. An icon is fetched as
/// a favicon (no cookies) and bounded to `MAX_ICON_PX`; a full image with the
/// page's cookies and no bound (`max_image_size` 0).
pub(crate) fn fetch(host: &impl cef::ImplBrowserHost, url: &str, sink: &Sink, purpose: Purpose) {
    let mut callback = FaviconCallback::new(sink.clone(), purpose);
    let (is_favicon, max) = match purpose {
        Purpose::Icon => (1, MAX_ICON_PX),
        Purpose::Full => (0, 0),
    };
    host.download_image(Some(&url.into()), is_favicon, max, 0, Some(&mut callback));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_every_size_and_shrinks_large_ones() {
        let ok = accept("u", 2, 3, vec![0; 24]).unwrap();
        assert_eq!((ok.width, ok.height, ok.rgba.len()), (2, 3, 24));
        assert!(accept("u", 64, 64, vec![0; 64 * 64 * 4]).is_some());
        let big = accept("u", 128, 128, vec![0; 128 * 128 * 4]).unwrap();
        assert_eq!(
            (big.width, big.height, big.rgba.len()),
            (64, 64, 64 * 64 * 4)
        );
        let wide = accept("u", 65, 1, vec![0; 65 * 4]).unwrap();
        assert_eq!((wide.width, wide.height), (64, 1));
        assert!(accept("u", 0, 3, vec![]).is_none());
        assert!(accept("u", -1, 3, vec![0; 12]).is_none());
        assert!(accept("u", 2, 3, vec![0; 23]).is_none());
        assert!(accept("u", 2, 3, vec![]).is_none());
    }

    #[test]
    fn a_full_image_keeps_its_size_and_refuses_empty_wrong_or_huge_ones() {
        let ok = accept_full(300, 2, vec![0; 2400]).unwrap();
        assert_eq!((ok.width, ok.height, ok.rgba.len()), (300, 2, 2400));
        assert!(accept_full(0, 2, vec![]).is_err());
        assert!(accept_full(-1, 2, vec![0; 8]).is_err());
        assert!(accept_full(2, 2, vec![0; 15]).is_err());
        let side = 4097; // just over 16 Mpx
        let huge = accept_full(side, side, vec![0; (side * side * 4) as usize]);
        assert_eq!(huge, Err("the image is too large".into()));
    }
}
