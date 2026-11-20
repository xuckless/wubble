//! The scene's glass has exactly the owners Slint shows: after any sequence of overlays opening,
//! closing and replacing each other, at any frame timing, every glass panel on screen has one
//! rect in the list the scene holds, at its place and size, and no rect is left without a panel on
//! screen. Owner's report: right-clicking a tab while Settings was open showed the menu's rows
//! with no glass behind them (several menus share `GlassKeys.menu`: a closed Select's panel
//! reporting `present: false` took the open context menu's glass out). Testing backend, mock
//! time, explicit frames through `Wired::frame` (what the rendering notifier calls); one test,
//! one platform per process.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use i_slint_backend_testing::{
    ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::{ComponentHandle, LogicalSize, ModelRc, VecModel};
use wubble_core::pages::settings_page;
use wubble_ui::driver::{self, Wired};
use wubble_ui::settings::setting_items;
use wubble_ui::{
    AppWindow, GlassItem, GlassKeys, GlassLayer, InternalPageKind, MenuEntry, MenuView, TabItem,
};

/// The glass the scene holds and the frame clock.
struct Rig {
    app: AppWindow,
    wired: Wired,
    scene: Vec<GlassItem>,
    now: Instant,
}

impl Rig {
    /// One rendered frame `dt` after the last.
    fn frame(&mut self, dt: Duration) -> bool {
        mock_elapsed_time(dt);
        self.now += dt;
        let (again, out) = self.wired.frame(&self.app, self.now);
        if let Some(glass) = out {
            self.scene = glass;
        }
        again
    }

    /// Frames at 120 Hz until nothing asks for another and every curve is over.
    fn settle(&mut self) {
        for _ in 0..600 {
            if !self.frame(Duration::from_micros(8_333)) {
                break;
            }
        }
        // the tweens run on Slint's clock, not the driver's: give them their time, then the
        // frames their change
        mock_elapsed_time(Duration::from_millis(1_000));
        for _ in 0..600 {
            if !self.frame(Duration::from_micros(8_333)) {
                return;
            }
        }
        panic!("never settled");
    }

    /// The invariant, at rest: every overlay rect the scene holds is a glass panel Slint shows
    /// at that place and size, and every shown overlay panel the test knows (the context menu)
    /// has one.
    fn check(&self, what: &str) {
        let panels: Vec<_> = ElementHandle::find_by_element_type_name(&self.app, "GlassPanel")
            .filter(|p| p.computed_opacity() > 0.0)
            .map(|p| {
                let (a, s) = (p.absolute_position(), p.size());
                (a.x, a.y, s.width, s.height, p.accessible_label())
            })
            .filter(|p| p.2 > 0.0 && p.3 > 0.0)
            .collect();
        let near = |a: f32, b: f32| (a - b).abs() < 0.5;
        for item in self.scene.iter().filter(|i| i.layer > 0) {
            assert!(
                panels.iter().any(|p| near(p.0, item.x)
                    && near(p.1, item.y)
                    && near(p.2, item.width)
                    && near(p.3, item.height)),
                "{what}: glass {item:?} with no panel on screen there"
            );
        }
        let key = self.app.global::<GlassKeys>().get_menu();
        let menus: Vec<_> = self.scene.iter().filter(|i| i.key == key).collect();
        match panels.iter().find(|p| p.4.as_deref() == Some("Menu")) {
            Some(p) => assert!(
                menus.len() == 1
                    && near(menus[0].x, p.0)
                    && near(menus[0].y, p.1)
                    && near(menus[0].width, p.2)
                    && near(menus[0].height, p.3),
                "{what}: the menu shows at {p:?}, the scene's menu glass is {menus:?}"
            ),
            None => assert!(
                menus.is_empty(),
                "{what}: menu glass {menus:?} with no menu"
            ),
        }
    }
}

fn entry(label: &str) -> MenuEntry {
    MenuEntry {
        label: label.into(),
        enabled: true,
        ..Default::default()
    }
}

fn menu(x: f32, y: f32, rows: usize) -> MenuView {
    MenuView {
        open: true,
        x,
        y,
        icons: false,
        entries: ModelRc::from(Rc::new(VecModel::from(
            (0..rows)
                .map(|i| entry(&format!("Row {i} of the tab menu")))
                .collect::<Vec<_>>(),
        ))),
    }
}

/// A closed menu, as the host writes it (surfaces.rs `menu_view`: `MenuView::default()`).
fn closed() -> MenuView {
    MenuView::default()
}

/// Frame steps: a fixed 120 Hz, a slow 60 Hz, and an irregular pattern (a small LCG, so every run
/// is the same).
fn steps(kind: usize, seed: u32) -> impl FnMut() -> Duration {
    let mut s = seed.wrapping_mul(2_654_435_761).max(1);
    move || match kind {
        0 => Duration::from_micros(8_333),
        1 => Duration::from_micros(16_667),
        _ => {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            Duration::from_millis(u64::from(1 + (s >> 16) % 90))
        }
    }
}

#[test]
fn every_glass_has_its_panel_and_every_panel_its_glass() {
    init_integration_test_with_mock_time();
    let app = AppWindow::new().unwrap();
    app.show().unwrap();
    app.window().set_size(LogicalSize::new(1280.0, 800.0));
    app.global::<GlassLayer>().set_scene_active(true);
    let wired = driver::wire(&app, wubble_core::seam::Seam::new());
    app.set_tabs(ModelRc::from(Rc::new(VecModel::from(
        (1..=4)
            .map(|id| TabItem {
                id,
                title: format!("Tab {id}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    ))));
    // Settings on screen: its Selects are kit menus too, closed, under the same key
    app.set_page(InternalPageKind::Settings);
    app.set_settings_rows(ModelRc::from(Rc::new(VecModel::from(setting_items(
        &settings_page(&HashMap::new()),
    )))));
    let mut rig = Rig {
        app,
        wired,
        scene: vec![],
        now: Instant::now(),
    };
    rig.settle();
    rig.check("at start");

    type Seq = (&'static str, fn(&mut Rig, &mut dyn FnMut() -> Duration));
    let sequences: [Seq; 6] = [
        ("open the tab menu", |r, _| {
            r.app.set_menu(menu(40.0, 120.0, 8));
        }),
        ("open, close, reopen at once", |r, step| {
            r.app.set_menu(menu(40.0, 120.0, 8));
            r.frame(step());
            r.frame(step());
            r.app.set_menu(closed());
            r.frame(step());
            r.app.set_menu(menu(40.0, 160.0, 8));
        }),
        ("open, then right-click another row", |r, step| {
            r.app.set_menu(menu(40.0, 120.0, 8));
            for _ in 0..4 {
                r.frame(step());
            }
            r.app.set_menu(menu(60.0, 300.0, 12));
        }),
        ("a menu replaced while it fades out", |r, step| {
            r.app.set_menu(menu(40.0, 120.0, 8));
            for _ in 0..30 {
                r.frame(step());
            }
            r.app.set_menu(closed());
            r.frame(step());
            r.frame(step());
            r.app.set_menu(menu(700.0, 200.0, 5));
        }),
        ("open, close within the fade, settle closed", |r, step| {
            r.app.set_menu(menu(40.0, 120.0, 8));
            r.frame(step());
            r.app.set_menu(closed());
        }),
        ("a toast while a menu opens", |r, step| {
            r.app.set_menu(menu(40.0, 120.0, 8));
            r.frame(step());
            r.app.set_toast_text("Link copied".into());
            r.app.set_toast_serial(r.app.get_toast_serial() + 1);
            r.frame(step());
        }),
    ];
    for round in 0..50u32 {
        for (name, seq) in &sequences {
            let mut step = steps(round as usize % 3, round);
            seq(&mut rig, &mut step);
            rig.settle();
            rig.check(&format!("{name}, round {round}, open"));
            // a model change while the menu stands (the shell asks for a sweep)
            rig.app.invoke_shell_model_changed();
            rig.settle();
            rig.check(&format!("{name}, round {round}, after a model change"));
            rig.app.set_menu(closed());
            rig.settle();
            rig.check(&format!("{name}, round {round}, closed"));
        }
    }
}
