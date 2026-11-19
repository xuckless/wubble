//! A page's accessibility tree, from Chromium to AccessKit.
//!
//! CEF gives a windowless browser Chromium's tree as values (`CefAccessibilityHandler`,
//! [`value`]); this crate keeps a mirror of it ([`PageTree`]) and turns each change into an
//! AccessKit [`TreeUpdate`] for a subtree of its own ([`tree_id_of`]), meant to hang under the
//! page pane's node in the shell's tree through a graft node ([`graft_node`]). Requests an
//! assistive technology makes of a page node come back as [`PageAction`]s the engine can carry
//! out (CEF has no accessibility action API for windowless browsers: a click is a mouse click
//! at the node, focus and scrolling are page scripts).
//!
//! Not wired yet (TODO.md, M1 "Accessibility tree"): Slint 1.19 keeps its AccessKit adapter
//! private, so the shell cannot graft this subtree. The role table follows the choices of
//! Slint's own mapping (i-slint-backend-winit 1.19 `accesskit.rs`, MIT OR GPL-3.0) and
//! Chromium's `ui/accessibility/ax_enums.mojom` names (BSD-3-Clause); no code is copied.

pub mod value;

use std::collections::{BTreeMap, BTreeSet};

use accesskit::{
    Action, ActionRequest, Node, NodeId, Rect, Role, Toggled, TreeId, TreeInfo, TreeUpdate, Uuid,
};

pub use value::{AxNode, AxValue, Bounds, LocationChange, Updates};

/// Chromium role name to AccessKit role. Roles not listed are generic containers, which
/// AccessKit's platform adapters flatten away (their children stay).
const ROLES: &[(&str, Role)] = &[
    ("rootWebArea", Role::RootWebArea),
    ("webArea", Role::RootWebArea),
    ("document", Role::Document),
    ("heading", Role::Heading),
    ("paragraph", Role::Paragraph),
    ("staticText", Role::Label),
    ("inlineTextBox", Role::TextRun),
    ("labelText", Role::Label),
    ("link", Role::Link),
    ("button", Role::Button),
    ("popUpButton", Role::Button),
    ("textField", Role::TextInput),
    ("searchBox", Role::SearchInput),
    ("textFieldWithComboBox", Role::ComboBox),
    ("comboBoxSelect", Role::ComboBox),
    ("checkBox", Role::CheckBox),
    ("radioButton", Role::RadioButton),
    ("switch", Role::Switch),
    ("image", Role::Image),
    ("list", Role::List),
    ("listItem", Role::ListItem),
    ("article", Role::Article),
    ("section", Role::Section),
    ("group", Role::Group),
    ("form", Role::Form),
    ("navigation", Role::Navigation),
    ("main", Role::Main),
    ("banner", Role::Banner),
    ("contentInfo", Role::ContentInfo),
    ("complementary", Role::Complementary),
    ("region", Role::Region),
    ("search", Role::Search),
    ("table", Role::Table),
    ("row", Role::Row),
    ("cell", Role::Cell),
    ("columnHeader", Role::ColumnHeader),
    ("rowHeader", Role::RowHeader),
];

/// Roles whose default action is a click.
const CLICKABLE: &[Role] = &[
    Role::Link,
    Role::Button,
    Role::CheckBox,
    Role::RadioButton,
    Role::Switch,
    Role::ComboBox,
];

pub fn role_of(chromium: &str) -> Role {
    ROLES
        .iter()
        .find(|(name, _)| *name == chromium)
        .map_or(Role::GenericContainer, |(_, role)| *role)
}

/// The AccessKit tree a Chromium tree becomes: a fixed function of its `AXTreeID` string, so
/// the same document keeps its subtree across updates (FNV-1a, two seeds, into a UUID).
pub fn tree_id_of(ax_tree_id: &str) -> TreeId {
    let fnv = |seed: u64| {
        ax_tree_id.bytes().fold(seed, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    };
    let (hi, lo) = (fnv(0xcbf2_9ce4_8422_2325), fnv(0x6c62_272e_07bb_0142));
    TreeId(Uuid::from_u64_pair(hi, lo))
}

/// The node that holds a page's subtree in the shell's tree (under the page pane).
pub fn graft_node(tree: TreeId) -> Node {
    let mut node = Node::new(Role::GenericContainer);
    node.set_tree_id(tree);
    node
}

/// What an assistive technology asked of a page node, as the engine can do it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageAction {
    /// A left click at this point (CSS pixels of the page).
    Click { x: f64, y: f64 },
    /// Focus the node (a page script; the node is Chromium's id).
    Focus(i32),
    /// Scroll the node into view (a page script).
    ScrollIntoView(i32),
}

/// The mirror of one page's Chromium tree.
#[derive(Default)]
pub struct PageTree {
    tree_id: String,
    root: Option<i32>,
    focus: Option<i32>,
    nodes: BTreeMap<i32, AxNode>,
}

impl PageTree {
    /// Applies a tree change; the AccessKit update for what changed. A change for another
    /// document (a navigation) starts the mirror over.
    pub fn apply(&mut self, updates: &Updates) -> Option<TreeUpdate> {
        if updates.tree_id != self.tree_id {
            *self = PageTree {
                tree_id: updates.tree_id.clone(),
                ..PageTree::default()
            };
        }
        let mut changed = BTreeSet::new();
        for change in &updates.changes {
            if let Some(clear) = change.node_id_to_clear {
                self.clear_children(clear);
            }
            if let Some(root) = change.root_id {
                self.root = Some(root);
            }
            if change.focus_id.is_some() {
                self.focus = change.focus_id;
            }
            for node in &change.nodes {
                changed.insert(node.id);
                self.nodes.insert(node.id, node.clone());
            }
        }
        self.prune();
        self.update_for(changed)
    }

    /// Applies location changes for this document; the update for the moved nodes and those
    /// placed relative to them.
    pub fn move_nodes(&mut self, changes: &[LocationChange]) -> Option<TreeUpdate> {
        let mut moved = BTreeSet::new();
        for c in changes.iter().filter(|c| c.tree_id == self.tree_id) {
            if let Some(node) = self.nodes.get_mut(&c.id) {
                node.location = c.location;
                moved.insert(c.id);
            }
        }
        if moved.is_empty() {
            return None;
        }
        self.update_for(moved)
    }

    /// What `request` asks, if it targets this tree's node.
    pub fn action(&self, request: &ActionRequest) -> Option<PageAction> {
        if request.target_tree != tree_id_of(&self.tree_id) {
            return None;
        }
        let id = i32::try_from(request.target_node.0).ok()?;
        let node = self.nodes.get(&id)?;
        match request.action {
            Action::Click => {
                let b = self.absolute(node);
                Some(PageAction::Click {
                    x: b.x + b.width / 2.0,
                    y: b.y + b.height / 2.0,
                })
            }
            Action::Focus => Some(PageAction::Focus(id)),
            Action::ScrollIntoView => Some(PageAction::ScrollIntoView(id)),
            _ => None,
        }
    }

    fn clear_children(&mut self, id: i32) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.child_ids.clear();
        }
        self.prune();
    }

    /// Drops the nodes no longer reachable from the root.
    fn prune(&mut self) {
        let Some(root) = self.root else { return };
        let mut keep = BTreeSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if keep.insert(id)
                && let Some(n) = self.nodes.get(&id)
            {
                stack.extend(&n.child_ids);
            }
        }
        self.nodes.retain(|id, _| keep.contains(id));
    }

    /// The update carrying `changed` and every node placed relative to one of them.
    fn update_for(&self, changed: BTreeSet<i32>) -> Option<TreeUpdate> {
        let root = self.root.filter(|r| self.nodes.contains_key(r))?;
        let ids: Vec<i32> = self
            .nodes
            .values()
            .filter(|n| changed.contains(&n.id) || self.containers(n).any(|c| changed.contains(&c)))
            .map(|n| n.id)
            .collect();
        let focus = self
            .focus
            .filter(|f| self.nodes.contains_key(f))
            .unwrap_or(root);
        Some(TreeUpdate {
            nodes: ids
                .into_iter()
                .map(|id| (node_id(id), self.node(&self.nodes[&id], id == root)))
                .collect(),
            tree: Some(TreeInfo::new(node_id(root))),
            tree_id: tree_id_of(&self.tree_id),
            focus: node_id(focus),
        })
    }

    /// `node`'s offset containers, nearest first (bounded, in case of a cycle).
    fn containers<'a>(&'a self, node: &AxNode) -> impl Iterator<Item = i32> + 'a {
        let mut next = node.location.offset_container;
        std::iter::from_fn(move || {
            let id = next?;
            next = self
                .nodes
                .get(&id)
                .and_then(|c| c.location.offset_container);
            Some(id)
        })
        .take(self.nodes.len())
    }

    /// `node`'s bounds in page coordinates: each container's origin less its scroll offset.
    fn absolute(&self, node: &AxNode) -> Bounds {
        let mut b = node.location.bounds;
        for id in self.containers(node) {
            let Some(c) = self.nodes.get(&id) else { break };
            b.x += c.location.bounds.x - f64::from(c.int("scrollX").unwrap_or(0));
            b.y += c.location.bounds.y - f64::from(c.int("scrollY").unwrap_or(0));
        }
        b
    }

    fn node(&self, n: &AxNode, is_root: bool) -> Node {
        let mut role = role_of(&n.role);
        if role == Role::TextInput && n.has_state("multiline") {
            role = Role::MultilineTextInput;
        }
        let mut node = Node::new(role);
        if let Some(name) = n.text("name") {
            // AccessKit's static text carries its text as the value (as Slint's Text does).
            if role == Role::Label {
                node.set_value(name);
            } else {
                node.set_label(name);
            }
        }
        if let Some(value) = n.text("value") {
            node.set_value(value);
        }
        if let Some(d) = n.text("description") {
            node.set_description(d);
        }
        if let Some(p) = n.text("placeholder") {
            node.set_placeholder(p);
        }
        if let Some(url) = n.text("url").filter(|_| role == Role::Link) {
            node.set_url(url);
        }
        if let Some(level) = n.int("hierarchicalLevel").filter(|l| *l > 0) {
            node.set_level(level as usize);
        }
        match n.text("checkedState") {
            Some("true") => node.set_toggled(Toggled::True),
            Some("false") => node.set_toggled(Toggled::False),
            Some("mixed") => node.set_toggled(Toggled::Mixed),
            _ => {}
        }
        match n.text("restriction") {
            Some("disabled") => node.set_disabled(),
            Some("readOnly") => node.set_read_only(),
            _ => {}
        }
        if n.has_state("expanded") {
            node.set_expanded(true);
        } else if n.has_state("collapsed") {
            node.set_expanded(false);
        }
        if n.has_state("invisible") {
            node.set_hidden();
        }
        if n.has_state("required") {
            node.set_required();
        }
        if n.has_state("visited") {
            node.set_visited();
        }
        if n.flag("selected") {
            node.set_selected(true);
        }
        let b = self.absolute(n);
        node.set_bounds(Rect::new(b.x, b.y, b.x + b.width, b.y + b.height));
        node.set_children(
            n.child_ids
                .iter()
                .filter(|c| self.nodes.contains_key(c))
                .map(|c| node_id(*c))
                .collect::<Vec<_>>(),
        );
        if CLICKABLE.contains(&role) || n.actions.iter().any(|a| a == "doDefault") {
            node.add_action(Action::Click);
        }
        if n.has_state("focusable") {
            node.add_action(Action::Focus);
        }
        if !is_root {
            node.add_action(Action::ScrollIntoView);
        }
        node
    }
}

fn node_id(id: i32) -> NodeId {
    NodeId(u64::from(id.unsigned_abs()))
}

#[cfg(test)]
mod tests;
