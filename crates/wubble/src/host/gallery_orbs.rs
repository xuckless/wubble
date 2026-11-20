//! The agent widgets' gallery scenes (`gallery_smoke::Overlay::Orbs`): the specimen
//! (`kit/specimen/agent-widgets.slint`) filled with every thinking orb state, its orbs handed to
//! the window's orb stage (`crate::orbs`), which draws them. Any other scene takes them away.

use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, VecModel};
use wubble_core::orb::{Orb, OrbSize, OrbState, OrbTheme};
use wubble_ui::{AgentWidgets, AppWindow, OrbCell, OrbColumn, OrbGround, OrbRow, OrbTuned};

use super::Host;
use crate::gallery_smoke::{Ground, OrbSheet, Overlay};
use crate::orbs::{self, Shown};

/// The sides of the sizes sheet, logical px, each with the tuned design it shows: the icon
/// sizes from the 20 design (16 beside a tab title or a Space, 24 in a list row), then the three
/// tuned sizes as designed.
const SIZES: [(f32, OrbSize); 5] = [
    (16.0, OrbSize::S20),
    (20.0, OrbSize::S20),
    (24.0, OrbSize::S20),
    (32.0, OrbSize::S32),
    (64.0, OrbSize::S64),
];
/// The large sheet's side (a start page's or an empty state's orb), from the 64 design.
const LARGE: f32 = 160.0;
/// The large sheet's orbs per row.
const LARGE_ROW: usize = 3;

fn tuned(size: OrbSize) -> OrbTuned {
    match size {
        OrbSize::S64 => OrbTuned::S64,
        OrbSize::S32 => OrbTuned::S32,
        OrbSize::S20 => OrbTuned::S20,
    }
}

/// The specimen's columns and the orbs to draw, in slot order.
fn sheet(sheet: OrbSheet, page: Ground) -> (Vec<OrbColumn>, Vec<Shown>) {
    let paused = sheet == OrbSheet::Paused;
    // the substrate under each ground: dark ink is for light grounds
    let page_theme = if page == Ground::Dark {
        OrbTheme::Dark
    } else {
        OrbTheme::Light
    };
    let grounds: &[(OrbGround, OrbTheme)] = match sheet {
        OrbSheet::Sizes | OrbSheet::Paused => &[
            (OrbGround::Glass, OrbTheme::Dark),
            (OrbGround::Stage, OrbTheme::Dark),
            (OrbGround::Paper, OrbTheme::Light),
            (OrbGround::Page, page_theme),
        ],
        OrbSheet::Large => &[
            (OrbGround::Stage, OrbTheme::Dark),
            (OrbGround::Paper, OrbTheme::Light),
        ],
    };
    let mut shown = vec![];
    let mut cell = |state: OrbState, side: f32, size: OrbSize, theme: OrbTheme| {
        shown.push(Shown {
            orb: Orb::new(state, size, theme),
            side,
            paused,
        });
        OrbCell {
            slot: (shown.len() - 1) as i32,
            label: state.label().into(),
            tuned: tuned(size),
            size: side,
        }
    };
    let mut columns = vec![];
    for &(ground, theme) in grounds {
        let rows: Vec<OrbRow> = match sheet {
            OrbSheet::Sizes | OrbSheet::Paused => OrbState::ALL
                .into_iter()
                .map(|state| OrbRow {
                    name: state.label().into(),
                    cells: ModelRc::from(Rc::new(VecModel::from(
                        SIZES
                            .iter()
                            .map(|&(side, size)| cell(state, side, size, theme))
                            .collect::<Vec<_>>(),
                    ))),
                })
                .collect(),
            OrbSheet::Large => OrbState::ALL
                .chunks(LARGE_ROW)
                .map(|states| OrbRow {
                    name: Default::default(),
                    cells: ModelRc::from(Rc::new(VecModel::from(
                        states
                            .iter()
                            .map(|&state| cell(state, LARGE, OrbSize::S64, theme))
                            .collect::<Vec<_>>(),
                    ))),
                })
                .collect(),
        };
        columns.push(OrbColumn {
            ground,
            light: theme == OrbTheme::Light,
            rows: ModelRc::from(Rc::new(VecModel::from(rows))),
        });
    }
    (columns, shown)
}

/// `WUBBLE_MEASURE_ORB=<state>` (hand runs, docs/measurements.md): one orb of that state at 64
/// on the library's stage, moving, over a new window, so `--measure idle` reports what one
/// animating orb costs. Nothing when the variable is unset or names no state.
pub(crate) fn measure_one(id: wubble_core::window::WindowId, window: &AppWindow) {
    let Some(state) = std::env::var("WUBBLE_MEASURE_ORB")
        .ok()
        .and_then(|v| OrbState::ALL.into_iter().find(|s| s.name() == v))
    else {
        return;
    };
    let cell = OrbCell {
        slot: 0,
        label: state.label().into(),
        tuned: OrbTuned::S64,
        size: 64.0,
    };
    let shown = [Shown {
        orb: Orb::new(state, OrbSize::S64, OrbTheme::Dark),
        side: 64.0,
        paused: false,
    }];
    let row = OrbRow {
        name: state.label().into(),
        cells: ModelRc::from(Rc::new(VecModel::from(vec![cell]))),
    };
    let column = OrbColumn {
        ground: OrbGround::Stage,
        light: false,
        rows: ModelRc::from(Rc::new(VecModel::from(vec![row]))),
    };
    let widgets = window.global::<AgentWidgets>();
    widgets.set_columns(ModelRc::from(Rc::new(VecModel::from(vec![column]))));
    orbs::show(id, window, &shown);
    widgets.set_specimen(true);
    println!("measure: one {} orb at 64, moving", state.name());
}

impl Host {
    /// Shows the scene's specimen, or takes it away.
    pub(super) fn gallery_orbs(&mut self, window: &AppWindow, overlay: Overlay, ground: Ground) {
        let widgets = window.global::<AgentWidgets>();
        let Overlay::Orbs(which) = overlay else {
            if widgets.get_specimen() {
                orbs::clear(self.w.id);
                widgets.set_specimen(false);
            }
            return;
        };
        let (columns, shown) = sheet(which, ground);
        widgets.set_columns(ModelRc::from(Rc::new(VecModel::from(columns))));
        orbs::show(self.w.id, window, &shown);
        widgets.set_specimen(true);
        let (n, moving) = orbs::counts(self.w.id);
        println!("smoke: gallery orbs {n} shown, {moving} moving");
    }
}
