//! One windowless browser: frames, resize, HiDPI scale, close.
//!
//! The lifecycle is the pure [`transition`]; [`View`] wraps welding's
//! `PlatformCefProducer` (which owns the CEF render handler and the dmabuf
//! custody) and runs the effects. Views are owned by [`crate::app::Engine`],
//! which closes them on shutdown and reports each one closed to its lifecycle.

use cef::{ImplBrowser, ImplBrowserHost};
use welding::{
    CefRuntime, CefSurfaceConfig, CefSurfaceProducer, PlatformCefConfig, PlatformCefProducer,
};
use wubble_gfx::{FrameSize, NativeFrame};

use crate::app::EngineError;

/// Physical size and device scale of the rectangle a view fills.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub size: FrameSize,
    pub scale: f32,
}

impl Layout {
    /// The layout CEF can use: at least 1x1 pixels.
    fn clamped(self) -> Self {
        Self {
            size: self.size.at_least_one(),
            ..self
        }
    }
}

/// Where a view is in its life.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewState {
    /// CEF paints at `layout`.
    Live {
        layout: Layout,
    },
    /// CEF was told about `wanted`; frames at the old size may still arrive.
    Resizing {
        wanted: Layout,
    },
    /// Close was requested; waiting for CEF to drop the browser.
    Closing,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewEvent {
    /// The host's content rectangle or scale factor (possibly unchanged).
    Layout(Layout),
    /// CEF delivered a frame of this physical size.
    Frame(FrameSize),
    CloseRequested,
    /// CEF no longer knows the browser.
    GoneFromCef,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewEffect {
    Resize(FrameSize),
    SetScale(f32),
    /// Hand the frame to the host.
    Present,
    Close,
    /// Tell the engine lifecycle this browser is gone.
    ReportClosed,
}

/// The layout CEF is painting at or was last asked for.
fn target(state: ViewState) -> Option<Layout> {
    match state {
        ViewState::Live { layout } => Some(layout),
        ViewState::Resizing { wanted } => Some(wanted),
        ViewState::Closing | ViewState::Closed => None,
    }
}

/// Every (state, event) pair not matched is a no-op.
pub fn transition(state: ViewState, event: ViewEvent) -> (ViewState, Vec<ViewEffect>) {
    use {ViewEffect::*, ViewEvent::*, ViewState::*};
    match (state, event, target(state)) {
        (Live { .. } | Resizing { .. }, Layout(new), Some(current)) => {
            let new = new.clamped();
            if new == current {
                return (state, vec![]);
            }
            let mut effects = vec![];
            if new.size != current.size {
                effects.push(Resize(new.size));
            }
            if new.scale != current.scale {
                effects.push(SetScale(new.scale));
            }
            (Resizing { wanted: new }, effects)
        }
        // Frames at a stale size are still shown (stretched) rather than
        // dropped, so a resize never flashes an empty view.
        (Live { .. }, Frame(_), _) => (state, vec![Present]),
        (Resizing { wanted }, Frame(size), _) if size == wanted.size => {
            (Live { layout: wanted }, vec![Present])
        }
        (Resizing { .. }, Frame(_), _) => (state, vec![Present]),
        (Live { .. } | Resizing { .. }, CloseRequested, _) => (Closing, vec![Close]),
        (Closing, GoneFromCef, _) => (Closed, vec![ReportClosed]),
        _ => (state, vec![]),
    }
}

/// What a new view starts with.
#[derive(Clone, Debug)]
pub struct ViewConfig {
    pub url: String,
    pub layout: Layout,
    /// CEF `windowless_frame_rate`; see `wubble_gfx::pacing::frame_rate_for`.
    pub frame_rate: u32,
}

/// One live windowless browser.
pub struct View {
    producer: PlatformCefProducer,
    /// Saved at creation: welding zeroes its own copy on close, and the id is
    /// how a closed browser is detected (CEF stops resolving it).
    browser_id: i32,
    state: ViewState,
}

impl View {
    pub(crate) fn open(runtime: &CefRuntime, config: ViewConfig) -> Result<Self, EngineError> {
        let layout = config.layout.clamped();
        let producer = PlatformCefProducer::new(
            runtime,
            PlatformCefConfig {
                surface: CefSurfaceConfig {
                    initial_url: config.url,
                    initial_size: dpi_size(layout.size),
                    scale_factor: layout.scale,
                    ..CefSurfaceConfig::default()
                },
            },
        )
        .map_err(|e| EngineError::View(e.to_string()))?;
        let browser_id = producer.browser_id();
        // welding creates the browser at a fixed 60 fps; apply ours now.
        let rate = i32::try_from(config.frame_rate).unwrap_or(i32::MAX);
        match cef::browser_host_get_browser_by_identifier(browser_id)
            .and_then(|browser| browser.host())
        {
            Some(host) => host.set_windowless_frame_rate(rate),
            None => eprintln!("wubble-engine: view {browser_id}: no host; frame rate stays 60"),
        }
        Ok(Self {
            producer,
            browser_id,
            state: ViewState::Live { layout },
        })
    }

    /// Follows the host's content rectangle; unchanged layouts cost nothing.
    pub fn set_layout(&mut self, layout: Layout) {
        self.apply(ViewEvent::Layout(layout));
    }

    /// The newest frame CEF painted since the last call, if it is to be shown.
    pub fn take_frame(&mut self) -> Option<NativeFrame> {
        let frame = NativeFrame::from(self.producer.acquire_native_frame()?);
        let effects = self.apply(ViewEvent::Frame(frame.size()));
        effects.contains(&ViewEffect::Present).then_some(frame)
    }

    pub(crate) fn close(&mut self) {
        self.apply(ViewEvent::CloseRequested);
    }

    /// While closing, checks whether CEF has let go of the browser. Returns
    /// true exactly once, when the view reports itself closed.
    pub(crate) fn poll_closed(&mut self) -> bool {
        if self.state != ViewState::Closing
            || cef::browser_host_get_browser_by_identifier(self.browser_id).is_some()
        {
            return false;
        }
        self.apply(ViewEvent::GoneFromCef)
            .contains(&ViewEffect::ReportClosed)
    }

    /// Runs the transition and its CEF effects. Returns the effects meant for
    /// the caller (`Present`, `ReportClosed`).
    fn apply(&mut self, event: ViewEvent) -> Vec<ViewEffect> {
        let (next, effects) = transition(self.state, event);
        self.state = next;
        effects
            .into_iter()
            .filter(|&effect| !self.execute(effect))
            .collect()
    }

    /// The only place view effects touch CEF. Returns false for effects the
    /// caller handles.
    fn execute(&mut self, effect: ViewEffect) -> bool {
        let result = match effect {
            ViewEffect::Resize(size) => self.producer.resize(dpi_size(size)),
            ViewEffect::SetScale(scale) => self.producer.set_scale_factor(scale),
            ViewEffect::Close => self.producer.close(),
            ViewEffect::Present | ViewEffect::ReportClosed => return false,
        };
        // welding's producer calls fail only without its `cef-runtime`
        // feature, which wubble always enables; log rather than unwind.
        if let Err(e) = result {
            eprintln!(
                "wubble-engine: view {}: {effect:?} failed: {e}",
                self.browser_id
            );
        }
        true
    }
}

fn dpi_size(size: FrameSize) -> dpi::PhysicalSize<u32> {
    dpi::PhysicalSize::new(size.width, size.height)
}

#[cfg(test)]
mod tests {
    use super::{ViewEffect::*, ViewEvent::*, ViewState::*, *};

    const SMALL: FrameSize = FrameSize::new(800, 600);
    const BIG: FrameSize = FrameSize::new(1024, 768);

    fn layout(size: FrameSize, scale: f32) -> super::Layout {
        super::Layout { size, scale }
    }

    fn live() -> ViewState {
        Live {
            layout: layout(SMALL, 1.0),
        }
    }

    fn states() -> Vec<ViewState> {
        vec![
            live(),
            Resizing {
                wanted: layout(BIG, 1.0),
            },
            Closing,
            Closed,
        ]
    }

    fn events() -> Vec<ViewEvent> {
        vec![
            Layout(layout(SMALL, 1.0)),
            Layout(layout(BIG, 1.0)),
            Layout(layout(SMALL, 2.0)),
            Layout(layout(FrameSize::new(0, 0), 1.0)),
            Frame(SMALL),
            Frame(BIG),
            CloseRequested,
            GoneFromCef,
        ]
    }

    fn expected(state: ViewState, event: ViewEvent) -> (ViewState, Vec<ViewEffect>) {
        let resizing = |size, scale| Resizing {
            wanted: layout(size, scale),
        };
        let one = FrameSize::new(1, 1);
        match (state, event) {
            (Live { .. }, Layout(l)) if l == layout(SMALL, 1.0) => (state, vec![]),
            (Live { .. }, Layout(l)) if l == layout(BIG, 1.0) => {
                (resizing(BIG, 1.0), vec![Resize(BIG)])
            }
            (Live { .. }, Layout(l)) if l == layout(SMALL, 2.0) => {
                (resizing(SMALL, 2.0), vec![SetScale(2.0)])
            }
            (Live { .. }, Frame(_)) => (state, vec![Present]),
            (Live { .. } | Resizing { .. }, CloseRequested) => (Closing, vec![Close]),
            (Resizing { .. }, Layout(l)) if l == layout(SMALL, 1.0) => {
                (resizing(SMALL, 1.0), vec![Resize(SMALL)])
            }
            (Resizing { .. }, Layout(l)) if l == layout(BIG, 1.0) => (state, vec![]),
            (Resizing { .. }, Layout(l)) if l == layout(SMALL, 2.0) => {
                (resizing(SMALL, 2.0), vec![Resize(SMALL), SetScale(2.0)])
            }
            // The zero size, clamped.
            (Live { .. } | Resizing { .. }, Layout(_)) => (resizing(one, 1.0), vec![Resize(one)]),
            (Resizing { .. }, Frame(BIG)) => (live_at(BIG), vec![Present]),
            (Resizing { .. }, Frame(_)) => (state, vec![Present]),
            (Closing, GoneFromCef) => (Closed, vec![ReportClosed]),
            _ => (state, vec![]),
        }
    }

    fn live_at(size: FrameSize) -> ViewState {
        Live {
            layout: layout(size, 1.0),
        }
    }

    #[test]
    fn table_covers_every_pair() {
        for state in states() {
            for event in events() {
                assert_eq!(
                    transition(state, event),
                    expected(state, event),
                    "{state:?} + {event:?}"
                );
            }
        }
    }

    fn run(start: ViewState, events: &[ViewEvent]) -> (ViewState, Vec<ViewEffect>) {
        events
            .iter()
            .fold((start, vec![]), |(state, mut all), &event| {
                let (next, effects) = transition(state, event);
                all.extend(effects);
                (next, all)
            })
    }

    #[test]
    fn repeated_layouts_resize_once() {
        let big = Layout(layout(BIG, 1.0));
        let (state, effects) = run(live(), &[big, big, big]);
        assert_eq!(
            state,
            Resizing {
                wanted: layout(BIG, 1.0)
            }
        );
        assert_eq!(effects, vec![Resize(BIG)]);
    }

    #[test]
    fn a_second_resize_replaces_the_pending_one() {
        let other = FrameSize::new(640, 480);
        let (state, effects) = run(
            live(),
            &[
                Layout(layout(BIG, 1.0)),
                Layout(layout(other, 1.0)),
                Frame(BIG),
            ],
        );
        assert_eq!(
            state,
            Resizing {
                wanted: layout(other, 1.0)
            }
        );
        assert_eq!(effects, vec![Resize(BIG), Resize(other), Present]);
        assert_eq!(transition(state, Frame(other)).0, live_at(other));
    }

    #[test]
    fn stale_frames_are_presented_until_the_size_matches() {
        let (state, effects) = run(
            live(),
            &[Layout(layout(BIG, 1.0)), Frame(SMALL), Frame(SMALL)],
        );
        assert_eq!(
            state,
            Resizing {
                wanted: layout(BIG, 1.0)
            }
        );
        assert_eq!(effects, vec![Resize(BIG), Present, Present]);
        assert_eq!(transition(state, Frame(BIG)), (live_at(BIG), vec![Present]));
    }

    #[test]
    fn closing_ignores_layout_and_frames() {
        for event in [Layout(layout(BIG, 2.0)), Frame(SMALL), CloseRequested] {
            assert_eq!(transition(Closing, event), (Closing, vec![]));
        }
    }

    #[test]
    fn report_closed_exactly_once_on_every_path() {
        let paths: [&[ViewEvent]; 3] = [
            &[CloseRequested, GoneFromCef, GoneFromCef],
            &[
                Layout(layout(BIG, 1.0)),
                CloseRequested,
                Frame(BIG),
                GoneFromCef,
            ],
            &[
                GoneFromCef,
                CloseRequested,
                CloseRequested,
                GoneFromCef,
                CloseRequested,
            ],
        ];
        for path in paths {
            let (state, effects) = run(live(), path);
            assert_eq!(state, Closed, "{path:?}");
            let reports = effects.iter().filter(|e| **e == ReportClosed).count();
            let closes = effects.iter().filter(|e| **e == Close).count();
            assert_eq!((reports, closes), (1, 1), "{path:?}");
        }
    }
}
