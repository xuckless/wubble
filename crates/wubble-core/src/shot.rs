//! Screenshots of the visible page: what is on screen, the whole page, a dragged region or one
//! element. Chromium takes them (`Page.captureScreenshot` over the DevTools protocol); this is
//! the lifecycle ([`Shot`], [`transition`]), the clip arithmetic and the file name.
//!
//! Design reference, a few lines re-expressed: puppeteer's clip convention
//! (`packages/puppeteer-core/src/cdp/Page.ts`, `_screenshot`, Apache-2.0): a capture's `clip` is
//! in document CSS pixels, so a rectangle in the viewport is moved by the visual viewport's page
//! offset (`visualViewport.pageLeft`, `pageTop`).

use serde_json::{Value, json};

/// A rectangle, in the unit its use names.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A dragged region smaller than this (logical px) on either side is a click, not a region.
pub const MIN_REGION: f64 = 8.0;

impl RectF {
    /// The region dragged from `a` to `b` (view logical px, either corner first), clamped to the
    /// view's `size`; `None` when it is too small to mean anything.
    pub fn from_drag(a: (f64, f64), b: (f64, f64), size: (f64, f64)) -> Option<RectF> {
        let clamp = |v: f64, max: f64| v.clamp(0.0, max.max(0.0));
        let (x0, x1) = (clamp(a.0.min(b.0), size.0), clamp(a.0.max(b.0), size.0));
        let (y0, y1) = (clamp(a.1.min(b.1), size.1), clamp(a.1.max(b.1), size.1));
        let r = RectF {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        };
        (r.w >= MIN_REGION && r.h >= MIN_REGION).then_some(r)
    }
}

/// Where the visual viewport sits in the document and how big it is (CSS px), and for an
/// element capture the element's box in the viewport: the measuring script's answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub page_left: f64,
    pub page_top: f64,
    pub width: f64,
    pub height: f64,
    pub element: Option<RectF>,
}

impl Metrics {
    /// The script's JSON (`{x, y, w, h, el: [x, y, w, h] | null}`); `None` when malformed.
    pub fn from_json(text: &str) -> Option<Metrics> {
        let v: Value = serde_json::from_str(text).ok()?;
        let num = |v: &Value| v.as_f64().filter(|n| n.is_finite());
        let element = match v.get("el") {
            Some(Value::Array(r)) if r.len() == 4 => Some(RectF {
                x: num(&r[0])?,
                y: num(&r[1])?,
                w: num(&r[2])?,
                h: num(&r[3])?,
            }),
            _ => None,
        };
        Some(Metrics {
            page_left: num(v.get("x")?)?,
            page_top: num(v.get("y")?)?,
            width: num(v.get("w")?)?,
            height: num(v.get("h")?)?,
            element,
        })
    }
}

/// The script that measures the page for a clip; for an element, the one under `at` (viewport
/// CSS px). Its answer is [`Metrics::from_json`]'s.
pub fn measure_script(at: Option<(f64, f64)>) -> String {
    let element = match at {
        Some((x, y)) => format!(
            "(function(){{const e=document.elementFromPoint({x},{y});\
             if(!e)return null;const r=e.getBoundingClientRect();\
             return [r.left,r.top,r.width,r.height];}})()"
        ),
        None => "null".into(),
    };
    format!(
        "(function(){{const v=window.visualViewport;\
         return {{x:v.pageLeft,y:v.pageTop,w:v.width,h:v.height,el:{element}}};}})()"
    )
}

/// The capture's clip in document CSS px, from a rectangle in the viewport in CSS px, cut to
/// the visual viewport (a capture inside the view only: the windowless view cannot grow).
/// `None` when nothing of it is in view.
pub fn clip(rect: RectF, m: &Metrics) -> Option<RectF> {
    let x0 = rect.x.max(0.0);
    let y0 = rect.y.max(0.0);
    let x1 = (rect.x + rect.w).min(m.width);
    let y1 = (rect.y + rect.h).min(m.height);
    (x1 - x0 >= 1.0 && y1 - y0 >= 1.0).then_some(RectF {
        x: x0 + m.page_left,
        y: y0 + m.page_top,
        w: x1 - x0,
        h: y1 - y0,
    })
}

/// A region dragged on the view (logical px) in the page's CSS px: logical px over the zoom.
pub fn region_css(region: RectF, zoom_percent: u32) -> RectF {
    let z = f64::from(zoom_percent.max(1)) / 100.0;
    RectF {
        x: region.x / z,
        y: region.y / z,
        w: region.w / z,
        h: region.h / z,
    }
}

/// A view point in physical px (a context menu's) in the page's viewport CSS px.
pub fn point_css(physical: (i32, i32), scale: f32, zoom_percent: u32) -> (f64, f64) {
    let f = f64::from(scale.max(0.01)) * f64::from(zoom_percent.max(1)) / 100.0;
    (f64::from(physical.0) / f, f64::from(physical.1) / f)
}

/// What to capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Capture {
    /// What the view shows.
    Visible,
    /// The whole document: Chromium measures it itself (`captureBeyondViewport`, no clip).
    FullPage,
    /// A part of the view, document CSS px.
    Clip(RectF),
}

/// `Page.captureScreenshot`'s parameters for `capture`.
pub fn params(capture: Capture) -> Value {
    match capture {
        Capture::Visible => json!({ "format": "png" }),
        Capture::FullPage => json!({ "format": "png", "captureBeyondViewport": true }),
        Capture::Clip(r) => json!({
            "format": "png",
            "captureBeyondViewport": false,
            "clip": { "x": r.x, "y": r.y, "width": r.w, "height": r.h, "scale": 1 },
        }),
    }
}

/// Where a screenshot goes (`settings::SCREENSHOT_OUTPUT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotOutput {
    Both,
    File,
    Clipboard,
}

pub const OUTPUT_WORDS: [&str; 3] = ["both", "file", "clipboard"];

impl ShotOutput {
    pub const ALL: [ShotOutput; 3] = [Self::Both, Self::File, Self::Clipboard];

    pub fn id(self) -> &'static str {
        OUTPUT_WORDS[self as usize]
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.id() == id)
    }

    pub fn file(self) -> bool {
        matches!(self, Self::Both | Self::File)
    }

    pub fn clipboard(self) -> bool {
        matches!(self, Self::Both | Self::Clipboard)
    }
}

/// The file a screenshot of a page titled `title`, taken at `stamp` (local time), is saved as,
/// before the downloads folder makes it unique and safe (`download::unique_name`).
pub fn file_name(title: &str, stamp: chrono::NaiveDateTime) -> String {
    let title = title.trim();
    let when = stamp.format("%Y-%m-%d at %H.%M.%S");
    if title.is_empty() {
        format!("Screenshot {when}.png")
    } else {
        format!("Screenshot {when} {title}.png")
    }
}

/// The toast after a screenshot: which outputs worked.
pub fn saved_text(file: Option<bool>, clipboard: Option<bool>) -> &'static str {
    match (file, clipboard) {
        (Some(true), Some(true)) => "Screenshot saved to Downloads and copied",
        (Some(true), Some(false)) => "Screenshot saved to Downloads, not copied",
        (Some(true), None) => "Screenshot saved to Downloads",
        (Some(false), Some(true)) | (None, Some(true)) => "Screenshot copied",
        _ => "Could not save the screenshot",
    }
}

/// What a request asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShotRequest {
    Visible,
    FullPage,
    /// Drag a region on the overlay.
    Region,
    /// The element under this viewport point (CSS px), from the context menu.
    Element {
        x: f64,
        y: f64,
    },
}

/// One window's screenshot in flight: at most one at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Shot {
    #[default]
    Idle,
    /// The region overlay is up.
    Picking,
    /// Script `id` measures the page for `rect` (viewport CSS px) or, `None`, the element it
    /// was asked about.
    Measuring { id: u64, rect: Option<RectF> },
    /// The capture call `id` is out.
    Capturing(u64),
    /// The image is being written and copied.
    Saving,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShotEvent {
    Request(ShotRequest),
    /// The overlay's region, viewport CSS px.
    Picked(RectF),
    /// Escape on the overlay, or the page went away under it.
    Cancel,
    MeasureSent {
        id: u64,
        rect: Option<RectF>,
    },
    Measured(u64, Option<Metrics>),
    CaptureSent(u64),
    /// The capture call's answer: the PNG as base64, or the protocol's error.
    Captured(u64, Result<String, String>),
    /// Saving ended; the toast's text.
    Saved(&'static str),
    /// The engine could not take a call or a script.
    SendFailed,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShotEffect {
    ShowOverlay,
    HideOverlay,
    /// Run [`measure_script`] for an element at this point (or none), then `MeasureSent`.
    Measure {
        at: Option<(f64, f64)>,
        rect: Option<RectF>,
    },
    /// Send the capture, then `CaptureSent`.
    Capture(Capture),
    /// Decode, write and copy the PNG (base64), then `Saved`.
    Save(String),
    Toast(&'static str),
}

pub fn transition(state: Shot, event: ShotEvent) -> (Shot, Vec<ShotEffect>) {
    use ShotEffect as E;
    use ShotEvent::*;
    const FAILED: &str = "Could not take the screenshot";
    match (state, event) {
        (Shot::Idle, Request(ShotRequest::Visible)) => {
            (Shot::Idle, vec![E::Capture(Capture::Visible)])
        }
        (Shot::Idle, Request(ShotRequest::FullPage)) => {
            (Shot::Idle, vec![E::Capture(Capture::FullPage)])
        }
        (Shot::Idle, Request(ShotRequest::Region)) => (Shot::Picking, vec![E::ShowOverlay]),
        (Shot::Idle, Request(ShotRequest::Element { x, y })) => (
            Shot::Idle,
            vec![E::Measure {
                at: Some((x, y)),
                rect: None,
            }],
        ),
        // The overlay's own buttons.
        (Shot::Picking, Request(ShotRequest::Visible)) => (
            Shot::Idle,
            vec![E::HideOverlay, E::Capture(Capture::Visible)],
        ),
        (Shot::Picking, Request(ShotRequest::FullPage)) => (
            Shot::Idle,
            vec![E::HideOverlay, E::Capture(Capture::FullPage)],
        ),
        (Shot::Picking, Request(ShotRequest::Region)) | (Shot::Picking, Cancel) => {
            (Shot::Idle, vec![E::HideOverlay])
        }
        (Shot::Picking, Picked(rect)) => (
            Shot::Idle,
            vec![
                E::HideOverlay,
                E::Measure {
                    at: None,
                    rect: Some(rect),
                },
            ],
        ),
        (_, Request(_)) => (state, vec![E::Toast("A screenshot is already being taken")]),
        (Shot::Idle, MeasureSent { id, rect }) => (Shot::Measuring { id, rect }, vec![]),
        (Shot::Measuring { id: want, rect }, Measured(id, metrics)) if id == want => {
            let clipped = metrics.and_then(|m| clip(rect.or(m.element)?, &m));
            match clipped {
                Some(r) => (Shot::Idle, vec![E::Capture(Capture::Clip(r))]),
                None => (Shot::Idle, vec![E::Toast(FAILED)]),
            }
        }
        (Shot::Idle, CaptureSent(id)) => (Shot::Capturing(id), vec![]),
        (Shot::Capturing(want), Captured(id, result)) if id == want => match result {
            Ok(png) => (Shot::Saving, vec![E::Save(png)]),
            Err(_) => (Shot::Idle, vec![E::Toast(FAILED)]),
        },
        (Shot::Saving, Saved(text)) => (Shot::Idle, vec![E::Toast(text)]),
        (Shot::Idle | Shot::Measuring { .. } | Shot::Capturing(_), SendFailed) => {
            (Shot::Idle, vec![E::Toast(FAILED)])
        }
        // Stale answers; a cancel with no overlay up.
        (state, _) => (state, vec![]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ShotEffect as E;
    use ShotEvent::*;

    fn r(x: f64, y: f64, w: f64, h: f64) -> RectF {
        RectF { x, y, w, h }
    }

    fn metrics(page_left: f64, page_top: f64) -> Metrics {
        Metrics {
            page_left,
            page_top,
            width: 800.0,
            height: 600.0,
            element: None,
        }
    }

    #[test]
    fn a_drag_is_normalised_clamped_and_needs_some_size() {
        let view = (800.0, 600.0);
        assert_eq!(
            RectF::from_drag((300.0, 200.0), (100.0, 50.0), view),
            Some(r(100.0, 50.0, 200.0, 150.0))
        );
        assert_eq!(
            RectF::from_drag((700.0, 500.0), (900.0, 700.0), view),
            Some(r(700.0, 500.0, 100.0, 100.0))
        );
        assert_eq!(
            RectF::from_drag((-20.0, 10.0), (40.0, 30.0), view),
            Some(r(0.0, 10.0, 40.0, 20.0))
        );
        assert_eq!(RectF::from_drag((10.0, 10.0), (15.0, 300.0), view), None);
        assert_eq!(RectF::from_drag((10.0, 10.0), (10.0, 10.0), view), None);
    }

    #[test]
    fn a_region_moves_by_the_scroll_and_divides_by_the_zoom() {
        let css = region_css(r(125.0, 250.0, 250.0, 125.0), 125);
        assert_eq!(css, r(100.0, 200.0, 200.0, 100.0));
        assert_eq!(
            clip(css, &metrics(0.0, 1500.0)),
            Some(r(100.0, 1700.0, 200.0, 100.0))
        );
        // Cut to the viewport; nothing in view is no clip.
        assert_eq!(
            clip(r(700.0, 550.0, 300.0, 300.0), &metrics(10.0, 20.0)),
            Some(r(710.0, 570.0, 100.0, 50.0))
        );
        assert_eq!(clip(r(900.0, 0.0, 50.0, 50.0), &metrics(0.0, 0.0)), None);
    }

    #[test]
    fn a_menu_point_is_in_css_px() {
        assert_eq!(point_css((300, 150), 1.5, 100), (200.0, 100.0));
        assert_eq!(point_css((250, 125), 1.0, 125), (200.0, 100.0));
    }

    #[test]
    fn metrics_parse_from_the_script() {
        let m = Metrics::from_json(r#"{"x":0,"y":12.5,"w":800,"h":600,"el":[1,2,3,4]}"#).unwrap();
        assert_eq!(m.page_top, 12.5);
        assert_eq!(m.element, Some(r(1.0, 2.0, 3.0, 4.0)));
        let m = Metrics::from_json(r#"{"x":0,"y":0,"w":800,"h":600,"el":null}"#).unwrap();
        assert_eq!(m.element, None);
        assert_eq!(Metrics::from_json("null"), None);
        assert!(measure_script(Some((1.0, 2.0))).contains("elementFromPoint(1,2)"));
        assert!(measure_script(None).contains("el:null"));
    }

    #[test]
    fn capture_parameters() {
        assert_eq!(params(Capture::Visible), json!({"format":"png"}));
        assert_eq!(
            params(Capture::FullPage),
            json!({"format":"png","captureBeyondViewport":true})
        );
        assert_eq!(
            params(Capture::Clip(r(1.0, 2.0, 3.0, 4.0)))["clip"],
            json!({"x":1.0,"y":2.0,"width":3.0,"height":4.0,"scale":1})
        );
    }

    #[test]
    fn names_and_outputs() {
        let at = chrono::NaiveDate::from_ymd_opt(2026, 11, 18)
            .unwrap()
            .and_hms_opt(9, 5, 7)
            .unwrap();
        assert_eq!(
            file_name(" Example ", at),
            "Screenshot 2026-11-18 at 09.05.07 Example.png"
        );
        assert_eq!(file_name("", at), "Screenshot 2026-11-18 at 09.05.07.png");
        for o in ShotOutput::ALL {
            assert_eq!(ShotOutput::from_id(o.id()), Some(o));
            assert!(o.file() || o.clipboard());
        }
        assert_eq!(
            saved_text(Some(true), Some(true)),
            "Screenshot saved to Downloads and copied"
        );
        assert_eq!(saved_text(Some(false), Some(true)), "Screenshot copied");
        assert_eq!(
            saved_text(Some(false), None),
            "Could not save the screenshot"
        );
    }

    #[test]
    fn visible_and_full_page_capture_and_save() {
        let (s, e) = transition(Shot::Idle, Request(ShotRequest::FullPage));
        assert_eq!(e, vec![E::Capture(Capture::FullPage)]);
        let (s, _) = transition(s, CaptureSent(9));
        assert_eq!(s, Shot::Capturing(9));
        let (s, e) = transition(s, Captured(9, Ok("iVBO".into())));
        assert_eq!((s, e), (Shot::Saving, vec![E::Save("iVBO".into())]));
        let (s, e) = transition(s, Saved("ok"));
        assert_eq!((s, e), (Shot::Idle, vec![E::Toast("ok")]));
    }

    #[test]
    fn a_region_is_picked_measured_and_clipped() {
        let (s, e) = transition(Shot::Idle, Request(ShotRequest::Region));
        assert_eq!((s, e), (Shot::Picking, vec![E::ShowOverlay]));
        let region = r(10.0, 20.0, 100.0, 80.0);
        let (s, e) = transition(s, Picked(region));
        assert_eq!(
            e,
            vec![
                E::HideOverlay,
                E::Measure {
                    at: None,
                    rect: Some(region)
                }
            ]
        );
        let (s, _) = transition(
            s,
            MeasureSent {
                id: 3,
                rect: Some(region),
            },
        );
        let (s, e) = transition(s, Measured(3, Some(metrics(0.0, 100.0))));
        assert_eq!(s, Shot::Idle);
        assert_eq!(
            e,
            vec![E::Capture(Capture::Clip(r(10.0, 120.0, 100.0, 80.0)))]
        );
    }

    #[test]
    fn an_element_is_measured_by_the_page() {
        let (s, e) = transition(Shot::Idle, Request(ShotRequest::Element { x: 5.0, y: 6.0 }));
        assert_eq!(
            e,
            vec![E::Measure {
                at: Some((5.0, 6.0)),
                rect: None
            }]
        );
        let (s, _) = transition(s, MeasureSent { id: 4, rect: None });
        let m = Metrics {
            element: Some(r(0.0, 0.0, 50.0, 20.0)),
            ..metrics(0.0, 10.0)
        };
        let (_, e) = transition(s, Measured(4, Some(m)));
        assert_eq!(e, vec![E::Capture(Capture::Clip(r(0.0, 10.0, 50.0, 20.0)))]);
        // No element there, or no answer: nothing to capture.
        let s = Shot::Measuring { id: 4, rect: None };
        assert_eq!(
            transition(s, Measured(4, Some(metrics(0.0, 0.0)))),
            (Shot::Idle, vec![E::Toast("Could not take the screenshot")])
        );
        assert_eq!(
            transition(s, Measured(4, None)).1,
            vec![E::Toast("Could not take the screenshot")]
        );
    }

    #[test]
    fn busy_ignores_escape_cancels_and_stale_answers_are_dropped() {
        for busy in [
            Shot::Capturing(1),
            Shot::Saving,
            Shot::Measuring { id: 1, rect: None },
        ] {
            assert_eq!(
                transition(busy, Request(ShotRequest::Visible)),
                (busy, vec![E::Toast("A screenshot is already being taken")])
            );
        }
        assert_eq!(
            transition(Shot::Picking, Cancel),
            (Shot::Idle, vec![E::HideOverlay])
        );
        assert_eq!(transition(Shot::Idle, Cancel), (Shot::Idle, vec![]));
        let s = Shot::Capturing(2);
        assert_eq!(transition(s, Captured(1, Ok(String::new()))), (s, vec![]));
        let s = Shot::Measuring { id: 2, rect: None };
        assert_eq!(transition(s, Measured(1, None)), (s, vec![]));
        assert_eq!(
            transition(Shot::Capturing(2), Captured(2, Err("no".into()))),
            (Shot::Idle, vec![E::Toast("Could not take the screenshot")])
        );
        assert_eq!(transition(Shot::Capturing(2), SendFailed).0, Shot::Idle);
    }
}
