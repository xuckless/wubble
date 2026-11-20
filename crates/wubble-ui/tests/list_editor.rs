//! The kit list editor on the testing backend, against its machine in Rust
//! (`wubble_core::controls::list_editor`): the same scripts of events, given to the widget as
//! clicks and keys and to `transition` as values, end in the same mode and selection with the
//! same requests to the owner (rows removed, drafts submitted, an add asked for). The widget runs
//! the table in Slint; this keeps the two one table. Mock time only; one test (one platform per
//! process).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use i_slint_backend_testing::{
    AccessibleRole, ElementHandle, init_integration_test_with_mock_time, mock_elapsed_time,
};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use wubble_core::controls::list_editor::{
    Adds, Caps, Effect, Event, ListEditor as Machine, Mode, transition,
};

slint::slint! {
    #[edition = "1.19"]
    import { ListEditor, ListEditorMode, ListEditorAdds } from "ui/kit/controls/list-editor.slint";
    import { EntryItem } from "ui/components/model.slint";
    export { EntryItem, ListEditorMode, ListEditorAdds }
    export component Specimen inherits Window {
        in property <[EntryItem]> rows;
        in property <ListEditorAdds> adds;
        in property <bool> confirm;
        in property <int> keep;
        in property <bool> editable: true;
        in property <int> answer;
        in property <string> note;
        out property <ListEditorMode> mode: le.mode;
        out property <int> selected: le.selected;
        out property <int> draft-row: le.state.row;
        out property <bool> refused: le.state.refused;
        out property <int> len: le.state.len;
        callback removed(string);
        callback submitted(string);
        callback ask-add();
        public function focus-list() {
            le.focus-list();
        }
        width: 640px;
        height: 480px;
        le := ListEditor {
            width: 600px;
            columns: ["Site", "Space", "Container"];
            count: root.rows.length;
            row-at(i) => {
                return root.rows[i];
            }
            adds: root.adds;
            confirm-remove: root.confirm;
            keep: root.keep;
            editable: root.editable;
            answer: root.answer;
            note: root.note;
            label: "Rules";
            confirm-note: "Press − again";
            removed(k) => {
                root.removed(k);
            }
            submitted(t) => {
                root.submitted(t);
            }
            ask-add => {
                root.ask-add();
            }
        }
    }
}

fn settle() {
    mock_elapsed_time(Duration::from_millis(400));
}

fn row(i: usize) -> EntryItem {
    EntryItem {
        label: format!("site{i}.test").into(),
        key: format!("site{i}.test").into(),
        cells: ModelRc::new(VecModel::from(vec![
            SharedString::from(format!("site{i}.test")),
            "Work".into(),
            "".into(),
        ])),
        ..Default::default()
    }
}

fn set_rows(app: &Specimen, n: usize) {
    app.set_rows(ModelRc::new(VecModel::from(
        (0..n).map(row).collect::<Vec<_>>(),
    )));
    settle();
}

fn key(app: &Specimen, k: impl Into<SharedString>) {
    let text: SharedString = k.into();
    app.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    app.window()
        .dispatch_event(WindowEvent::KeyReleased { text });
    settle();
}

fn button(app: &Specimen, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, label)
        .find(|h| h.accessible_role() == Some(AccessibleRole::Button))
        .unwrap_or_else(|| panic!("no {label} button"))
}

fn press(app: &Specimen, label: &str) {
    button(app, label).invoke_accessible_default_action();
    settle();
}

fn item(app: &Specimen, i: usize) -> ElementHandle {
    ElementHandle::find_by_accessible_label(app, &format!("site{i}.test"))
        .find(|h| h.accessible_role() == Some(AccessibleRole::ListItem))
        .unwrap_or_else(|| panic!("no row {i}"))
}

/// The widget's state as the machine's.
fn seen(app: &Specimen) -> Machine {
    let row = usize::try_from(app.get_draft_row()).ok();
    Machine {
        mode: match app.get_mode() {
            ListEditorMode::Browse => Mode::Browse,
            ListEditorMode::Draft => Mode::Draft {
                row,
                refused: app.get_refused(),
            },
            ListEditorMode::Sent => Mode::Sent { row },
            ListEditorMode::Asked => Mode::Asked,
            ListEditorMode::Confirm => Mode::Confirm,
        },
        selected: usize::try_from(app.get_selected()).ok(),
        len: usize::try_from(app.get_len()).unwrap(),
    }
}

/// Plays `events` on the widget and on the machine from `len` rows; both must agree after each.
fn play(app: &Specimen, caps: Caps, len: usize, events: &[Event]) -> Vec<Effect> {
    app.set_adds(match caps.adds {
        Adds::None => ListEditorAdds::None,
        Adds::Draft => ListEditorAdds::Draft,
        Adds::Owner => ListEditorAdds::Owner,
    });
    app.set_confirm(caps.confirm_remove);
    app.set_keep(caps.keep as i32);
    app.set_editable(caps.edits);
    // a fresh start: no rows, then `len`
    set_rows(app, 0);
    key(app, Key::Escape);
    set_rows(app, len);
    app.invoke_focus_list();
    let mut m = Machine::new(len);
    let fx = Rc::new(RefCell::new(Vec::<String>::new()));
    let f = fx.clone();
    app.on_removed(move |k| f.borrow_mut().push(format!("remove {k}")));
    let f = fx.clone();
    app.on_submitted(move |t| f.borrow_mut().push(format!("submit {t}")));
    let f = fx.clone();
    app.on_ask_add(move || f.borrow_mut().push("ask".into()));
    let mut want = vec![];
    for &e in events {
        match e {
            Event::Rows(n) => set_rows(app, n),
            Event::Pick(i) => {
                item(app, i).invoke_accessible_default_action();
                settle();
            }
            Event::Up => key(app, Key::UpArrow),
            Event::Down => key(app, Key::DownArrow),
            Event::Add => press(app, "Add"),
            Event::Edit(None) => key(app, Key::Return),
            Event::Edit(Some(_)) => unreachable!("a double click: not in these scripts"),
            Event::Remove => press(app, "Remove"),
            Event::Commit => key(app, Key::Return),
            Event::Typed => key(app, "x"),
            Event::Cancel => key(app, Key::Escape),
            Event::Answer { accepted, len } => {
                app.set_rows(ModelRc::new(VecModel::from(
                    (0..len).map(row).collect::<Vec<_>>(),
                )));
                app.set_note(if accepted { "" } else { "No" }.into());
                app.set_answer(app.get_answer() + 1);
                settle();
            }
        }
        let (next, effect) = transition(m, caps, e);
        m = next;
        want.extend(effect);
        assert_eq!(seen(app), m, "after {e:?} in {events:?}");
    }
    let got = fx.borrow().clone();
    assert_eq!(got.len(), want.len(), "effects {got:?} vs {want:?}");
    app.on_removed(|_| {});
    app.on_submitted(|_| {});
    app.on_ask_add(|| {});
    want
}

const LIST: Caps = Caps {
    adds: Adds::Draft,
    removes: true,
    edits: true,
    confirm_remove: false,
    keep: 0,
};
const SPACES: Caps = Caps {
    adds: Adds::Owner,
    removes: true,
    edits: true,
    confirm_remove: true,
    keep: 1,
};

#[test]
fn the_widget_runs_the_machine() {
    init_integration_test_with_mock_time();
    let app = Specimen::new().unwrap();
    app.show().unwrap();
    settle();

    play(&app, LIST, 3, &[Event::Down, Event::Down, Event::Up]);
    play(
        &app,
        LIST,
        3,
        &[Event::Pick(2), Event::Remove, Event::Rows(2)],
    );
    play(&app, LIST, 0, &[Event::Remove, Event::Down]);
    let fx = play(
        &app,
        LIST,
        2,
        &[
            Event::Add,
            Event::Typed,
            Event::Commit,
            Event::Answer {
                accepted: false,
                len: 2,
            },
            Event::Typed,
            Event::Commit,
            Event::Answer {
                accepted: true,
                len: 3,
            },
        ],
    );
    assert_eq!(fx, [Effect::Submit(None), Effect::Submit(None)]);
    play(
        &app,
        LIST,
        3,
        &[
            Event::Pick(1),
            Event::Edit(None),
            Event::Typed,
            Event::Commit,
            Event::Answer {
                accepted: true,
                len: 3,
            },
        ],
    );
    play(&app, LIST, 3, &[Event::Add, Event::Cancel, Event::Pick(0)]);
    play(
        &app,
        SPACES,
        2,
        &[Event::Add, Event::Rows(3), Event::Cancel],
    );
    play(
        &app,
        SPACES,
        2,
        &[
            Event::Pick(1),
            Event::Remove,
            Event::Cancel,
            Event::Remove,
            Event::Remove,
        ],
    );
    play(&app, SPACES, 1, &[Event::Pick(0), Event::Remove]);

    // What the owner gets: the row's key, the draft's cells (an edit after the row's key).
    let got = Rc::new(RefCell::new(Vec::<String>::new()));
    let g = got.clone();
    app.on_submitted(move |t| g.borrow_mut().push(t.to_string()));
    app.set_adds(ListEditorAdds::Draft);
    app.set_confirm(false);
    app.set_keep(0);
    set_rows(&app, 2);
    item(&app, 1).invoke_accessible_default_action();
    key(&app, Key::Return);
    key(&app, Key::Return);
    let draft = got.borrow()[0].clone();
    let decoded = wubble_core::pages::submission(&draft);
    assert_eq!(decoded.replacing, Some("site1.test"));
    assert_eq!(decoded.cells, ["site1.test", "Work", ""]);
    assert!(
        button(&app, "Remove").accessible_enabled() == Some(false),
        "a draft is out: − waits"
    );
    assert_eq!(app.get_rows().row_count(), 2);
}
