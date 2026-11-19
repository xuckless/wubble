//! Fixture trees in CEF's value shape (written as JSON, converted to [`AxValue`] as the engine
//! converts `cef::Value`).

use super::*;
use serde_json::{Value, json};
use value::{parse_locations, parse_updates};

fn ax(v: &Value) -> AxValue {
    match v {
        Value::Null => AxValue::Null,
        Value::Bool(b) => AxValue::Bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) => AxValue::Int(i as i32),
            None => AxValue::Double(n.as_f64().unwrap()),
        },
        Value::String(s) => AxValue::String(s.clone()),
        Value::Array(a) => AxValue::List(a.iter().map(ax).collect()),
        Value::Object(o) => AxValue::Dict(o.iter().map(|(k, v)| (k.clone(), ax(v))).collect()),
    }
}

/// A small page: a heading, a paragraph with a link, a text field, a button, inside a
/// scrolled container that offsets its children.
fn page() -> Value {
    json!({
        "ax_tree_id": "tree-a",
        "updates": [{
            "has_tree_data": true,
            "tree_data": { "tree_id": "tree-a", "title": "T", "loaded": true, "focus_id": 6 },
            "root_id": 1,
            "nodes": [
                { "id": 1, "role": "rootWebArea", "state": ["focusable"],
                  "location": { "x": 0.0, "y": 0.0, "width": 800.0, "height": 600.0 },
                  "child_ids": [2, 3],
                  "attributes": { "name": "Page title" } },
                { "id": 2, "role": "heading", "offset_container_id": 1,
                  "location": { "x": 10.0, "y": 20.0, "width": 300.0, "height": 40.0 },
                  "attributes": { "name": "Welcome", "hierarchicalLevel": 2 } },
                { "id": 3, "role": "genericContainer", "offset_container_id": 1,
                  "location": { "x": 0.0, "y": 100.0, "width": 800.0, "height": 400.0 },
                  "child_ids": [4, 6, 7],
                  "attributes": { "scrollY": 50 } },
                { "id": 4, "role": "paragraph", "offset_container_id": 3,
                  "location": { "x": 5.0, "y": 60.0, "width": 400.0, "height": 20.0 },
                  "child_ids": [5] },
                { "id": 5, "role": "link", "state": ["focusable", "visited"], "offset_container_id": 3,
                  "location": { "x": 5.0, "y": 60.0, "width": 80.0, "height": 20.0 },
                  "actions": ["doDefault"],
                  "attributes": { "name": "More", "url": "https://example.test/more" } },
                { "id": 6, "role": "textField", "state": ["focusable", "editable"],
                  "offset_container_id": 3,
                  "location": { "x": 5.0, "y": 100.0, "width": 200.0, "height": 24.0 },
                  "attributes": { "name": "Email", "value": "a@b", "placeholder": "you@" } },
                { "id": 7, "role": "button", "state": ["focusable"], "offset_container_id": 3,
                  "location": { "x": 5.0, "y": 140.0, "width": 60.0, "height": 24.0 },
                  "attributes": { "name": "Send", "restriction": "disabled" } }
            ]
        }],
        "events": [{ "event_type": "loadComplete", "id": 1 }]
    })
}

fn first(page: &mut PageTree, v: Value) -> TreeUpdate {
    page.apply(&parse_updates(&ax(&v))).expect("an update")
}

fn by_id(update: &TreeUpdate, id: u64) -> &Node {
    &update
        .nodes
        .iter()
        .find(|(n, _)| n.0 == id)
        .unwrap_or_else(|| panic!("node {id} in the update"))
        .1
}

#[test]
fn roles_names_and_levels_map() {
    let mut tree = PageTree::default();
    let u = first(&mut tree, page());
    assert_eq!(u.nodes.len(), 7);
    assert_eq!(u.tree, Some(TreeInfo::new(NodeId(1))));
    assert_eq!(by_id(&u, 1).role(), Role::RootWebArea);
    assert_eq!(by_id(&u, 1).label(), Some("Page title"));
    let h = by_id(&u, 2);
    assert_eq!(
        (h.role(), h.label(), h.level()),
        (Role::Heading, Some("Welcome"), Some(2))
    );
    assert_eq!(by_id(&u, 3).role(), Role::GenericContainer);
    let link = by_id(&u, 5);
    assert_eq!(link.role(), Role::Link);
    assert_eq!(link.url(), Some("https://example.test/more"));
    assert!(link.is_visited());
    let field = by_id(&u, 6);
    assert_eq!(field.role(), Role::TextInput);
    assert_eq!(
        (field.label(), field.value(), field.placeholder()),
        (Some("Email"), Some("a@b"), Some("you@"))
    );
    let button = by_id(&u, 7);
    assert_eq!((button.role(), button.is_disabled()), (Role::Button, true));
}

#[test]
fn children_keep_their_order() {
    let mut tree = PageTree::default();
    let u = first(&mut tree, page());
    let ids = |id| {
        by_id(&u, id)
            .children()
            .iter()
            .map(|n| n.0)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(1), [2, 3]);
    assert_eq!(ids(3), [4, 6, 7]);
    assert_eq!(ids(4), [5]);
}

#[test]
fn bounds_add_the_offset_containers_less_their_scroll() {
    let mut tree = PageTree::default();
    let u = first(&mut tree, page());
    assert_eq!(
        by_id(&u, 2).bounds(),
        Some(Rect::new(10.0, 20.0, 310.0, 60.0))
    );
    // the link: (5, 60) in container 3, which sits at (0, 100) in the root and is scrolled 50
    assert_eq!(
        by_id(&u, 5).bounds(),
        Some(Rect::new(5.0, 110.0, 85.0, 130.0))
    );
}

#[test]
fn focus_comes_from_the_tree_data_and_stays() {
    let mut tree = PageTree::default();
    assert_eq!(first(&mut tree, page()).focus, NodeId(6));
    // a later update without tree data keeps the focus
    let u = first(
        &mut tree,
        json!({ "ax_tree_id": "tree-a", "updates": [{ "nodes": [
            { "id": 2, "role": "heading", "offset_container_id": 1,
              "location": { "x": 10.0, "y": 20.0, "width": 300.0, "height": 40.0 },
              "attributes": { "name": "Hello", "hierarchicalLevel": 2 } }
        ]}]}),
    );
    assert_eq!(u.focus, NodeId(6));
    assert_eq!(u.nodes.len(), 1, "only the changed node");
    assert_eq!(by_id(&u, 2).label(), Some("Hello"));
}

#[test]
fn actions_follow_role_and_state() {
    let mut tree = PageTree::default();
    let u = first(&mut tree, page());
    let link = by_id(&u, 5);
    assert!(link.supports_action(Action::Click));
    assert!(link.supports_action(Action::Focus));
    assert!(link.supports_action(Action::ScrollIntoView));
    let heading = by_id(&u, 2);
    assert!(!heading.supports_action(Action::Click));
    assert!(!heading.supports_action(Action::Focus));
    assert!(
        !by_id(&u, 1).supports_action(Action::ScrollIntoView),
        "not the root"
    );
}

#[test]
fn node_id_to_clear_drops_the_old_subtree() {
    let mut tree = PageTree::default();
    first(&mut tree, page());
    // container 3 is cleared and now holds only a new paragraph 8
    let u = first(
        &mut tree,
        json!({ "ax_tree_id": "tree-a", "updates": [{ "node_id_to_clear": 3, "nodes": [
            { "id": 3, "role": "genericContainer", "offset_container_id": 1,
              "location": { "x": 0.0, "y": 100.0, "width": 800.0, "height": 400.0 },
              "child_ids": [8] },
            { "id": 8, "role": "paragraph", "offset_container_id": 3,
              "location": { "x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0 } }
        ]}]}),
    );
    assert_eq!(
        by_id(&u, 3)
            .children()
            .iter()
            .map(|n| n.0)
            .collect::<Vec<_>>(),
        [8]
    );
    // the focused field went with the old subtree: focus falls back to the root
    assert_eq!(u.focus, NodeId(1));
    let nodes = &tree.trees["tree-a"].nodes;
    assert!(!nodes.contains_key(&6) && !nodes.contains_key(&5));
}

#[test]
fn the_subtree_id_is_stable_per_document() {
    assert_eq!(tree_id_of("tree-a"), tree_id_of("tree-a"));
    assert_ne!(tree_id_of("tree-a"), tree_id_of("tree-b"));
    assert_ne!(tree_id_of("tree-a"), TreeId::ROOT);
    let mut tree = PageTree::default();
    assert_eq!(first(&mut tree, page()).tree_id, tree_id_of("tree-a"));
    let graft = graft_node(tree_id_of("tree-a"));
    assert_eq!(graft.tree_id(), Some(tree_id_of("tree-a")));
}

#[test]
fn a_new_document_starts_the_mirror_over() {
    let mut tree = PageTree::default();
    first(&mut tree, page());
    let u = first(
        &mut tree,
        json!({ "ax_tree_id": "tree-b", "updates": [{ "root_id": 1, "nodes": [
            { "id": 1, "role": "rootWebArea", "attributes": { "name": "Next" } }
        ]}]}),
    );
    assert_eq!(u.tree_id, tree_id_of("tree-b"));
    assert_eq!(u.nodes.len(), 1);
    assert_eq!(tree.trees.len(), 1, "the old document is gone");
    assert_eq!(tree.trees["tree-b"].nodes.len(), 1);
    assert_eq!(tree.tree(), Some(tree_id_of("tree-b")));
}

#[test]
fn a_moved_container_moves_what_it_contains() {
    let mut tree = PageTree::default();
    first(&mut tree, page());
    let moves = parse_locations(&ax(&json!([
        { "id": 3, "ax_tree_id": "tree-a",
          "new_location": { "offset_container_id": 1,
                            "bounds": { "x": 0.0, "y": 200.0, "width": 800.0, "height": 400.0 } } },
        { "id": 3, "ax_tree_id": "another-frame",
          "new_location": { "bounds": { "x": 9.0, "y": 9.0, "width": 9.0, "height": 9.0 } } }
    ])));
    assert_eq!(moves.len(), 2);
    let mut updates = tree.move_nodes(&moves);
    assert_eq!(updates.len(), 1, "a frame not on this page moves nothing");
    let u = updates.remove(0);
    let mut ids: Vec<u64> = u.nodes.iter().map(|(n, _)| n.0).collect();
    ids.sort();
    assert_eq!(
        ids,
        [3, 4, 5, 6, 7],
        "the container and everything placed in it"
    );
    assert_eq!(
        by_id(&u, 5).bounds(),
        Some(Rect::new(5.0, 210.0, 85.0, 230.0))
    );
    assert!(tree.move_nodes(&[]).is_empty());
}

#[test]
fn requests_become_page_actions() {
    let mut tree = PageTree::default();
    first(&mut tree, page());
    let request = |action, tree_id, node| ActionRequest {
        action,
        target_tree: tree_id,
        target_node: NodeId(node),
        data: None,
    };
    let here = tree_id_of("tree-a");
    assert_eq!(
        tree.action(&request(Action::Click, here, 5)),
        Some(PageAction::Click { x: 45.0, y: 120.0 })
    );
    assert_eq!(
        tree.action(&request(Action::Focus, here, 6)),
        Some(PageAction::Focus {
            tree: "tree-a".into(),
            node: 6
        })
    );
    assert_eq!(
        tree.action(&request(Action::ScrollIntoView, here, 7)),
        Some(PageAction::ScrollIntoView {
            tree: "tree-a".into(),
            node: 7
        })
    );
    assert_eq!(tree.action(&request(Action::Click, TreeId::ROOT, 5)), None);
    assert_eq!(tree.action(&request(Action::Click, here, 99)), None);
    assert_eq!(tree.action(&request(Action::Expand, here, 5)), None);
}

#[test]
fn missing_keys_read_as_defaults() {
    let u = parse_updates(&ax(
        &json!({ "updates": [{ "nodes": [{ "id": 4 }, { "role": "x" }] }] }),
    ));
    assert_eq!(u.tree_id, "");
    assert_eq!(
        u.changes[0].nodes.len(),
        1,
        "a node without an id is skipped"
    );
    assert_eq!(u.changes[0].nodes[0].location, value::Location::default());
    assert_eq!(role_of("somethingNew"), Role::GenericContainer);
    assert!(parse_locations(&AxValue::Null).is_empty());
}

/// An iframe's document: its own tree, a child of `tree-a`, hosted by `tree-a`'s node 8.
fn frame() -> Value {
    json!({
        "ax_tree_id": "frame-1",
        "updates": [{
            "has_tree_data": true,
            "tree_data": { "tree_id": "frame-1", "parent_tree_id": "tree-a", "focus_id": 2 },
            "root_id": 1,
            "nodes": [
                { "id": 1, "role": "rootWebArea",
                  "location": { "x": 0.0, "y": 0.0, "width": 300.0, "height": 150.0 },
                  "child_ids": [2] },
                { "id": 2, "role": "button", "offset_container_id": 1,
                  "location": { "x": 10.0, "y": 10.0, "width": 40.0, "height": 20.0 },
                  "attributes": { "name": "Play" } }
            ]
        }]
    })
}

/// `tree-a` with node 8, an iframe at (400, 0) hosting `frame-1`, added under the root.
fn host_change() -> Value {
    json!({
        "ax_tree_id": "tree-a",
        "updates": [{
            "nodes": [
                { "id": 1, "role": "rootWebArea", "state": ["focusable"],
                  "location": { "x": 0.0, "y": 0.0, "width": 800.0, "height": 600.0 },
                  "child_ids": [2, 3, 8],
                  "attributes": { "name": "Page title" } },
                { "id": 8, "role": "iframe", "offset_container_id": 1,
                  "location": { "x": 400.0, "y": 0.0, "width": 300.0, "height": 150.0 },
                  "attributes": { "childTreeId": "frame-1" } }
            ]
        }]
    })
}

#[test]
fn frames_keep_their_own_subtree_grafted_at_their_host() {
    let mut tree = PageTree::default();
    // the frame's tree can come before the page's; main and frame updates interleave
    let f = first(&mut tree, frame());
    assert_eq!(f.tree_id, tree_id_of("frame-1"));
    first(&mut tree, page());
    let host = first(&mut tree, host_change());
    first(&mut tree, frame());
    assert_eq!(
        tree.trees.len(),
        2,
        "neither update wiped the other document"
    );
    assert_eq!(tree.trees["tree-a"].nodes.len(), 8);
    assert_eq!(tree.tree(), Some(tree_id_of("tree-a")));
    let graft = by_id(&host, 8);
    assert_eq!(graft.tree_id(), Some(tree_id_of("frame-1")));
    assert!(graft.children().is_empty());
    assert_eq!(by_id(&host, 1).children().len(), 3);

    // a click in the frame lands where the frame sits in the page
    let click = tree.action(&ActionRequest {
        action: Action::Click,
        target_tree: tree_id_of("frame-1"),
        target_node: NodeId(2),
        data: None,
    });
    assert_eq!(click, Some(PageAction::Click { x: 430.0, y: 20.0 }));

    // a frame's location change reaches only its own document
    let moves = parse_locations(&ax(&json!([
        { "id": 2, "ax_tree_id": "frame-1",
          "new_location": { "offset_container_id": 1,
                            "bounds": { "x": 0.0, "y": 0.0, "width": 40.0, "height": 20.0 } } }
    ])));
    let updates = tree.move_nodes(&moves);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].tree_id, tree_id_of("frame-1"));

    // navigating: a new parentless document drops the old page and its frames
    first(
        &mut tree,
        json!({ "ax_tree_id": "tree-b", "updates": [{ "root_id": 1, "nodes": [
            { "id": 1, "role": "rootWebArea" }
        ]}]}),
    );
    assert_eq!(tree.trees.keys().collect::<Vec<_>>(), ["tree-b"]);
}
